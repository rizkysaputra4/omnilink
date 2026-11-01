package com.omnilink.android

import android.Manifest
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Base64
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ExperimentalGetImage
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import com.google.gson.Gson
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import com.omnilink.android.network.TunnelWebSocketClient
import com.omnilink.android.terminal.TerminalScreen
import kotlinx.coroutines.launch
import java.util.concurrent.Executors

data class PairingData(
    val relay: String,
    val room: String,
    val host_pub: String,
    val auth_token: String,
    val pin: String
)

class TerminalTab(
    val id: Int,
    var title: String,
    val screen: TerminalScreen = TerminalScreen()
) {
    var revision by mutableStateOf(0)
}

data class PushNotificationMsg(
    val channel: String?,
    val title: String?,
    val message: String?,
    val priority: String?,
    val term_id: Int?
)

fun showAgentNotification(context: Context, title: String, message: String, termId: Int) {
    val channelId = "omnilink_agent_alerts"
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        val channel = android.app.NotificationChannel(
            channelId,
            "OmniLink AI Agent Alerts",
            android.app.NotificationManager.IMPORTANCE_HIGH
        ).apply {
            description = "Push alerts from AI agents running on host PC"
            enableVibration(true)
        }
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as android.app.NotificationManager
        manager.createNotificationChannel(channel)
    }

    val intent = Intent(context, MainActivity::class.java).apply {
        action = Intent.ACTION_VIEW
        flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        putExtra("EXTRA_TARGET_TERM_ID", termId)
    }

    val pendingIntent = android.app.PendingIntent.getActivity(
        context,
        termId,
        intent,
        android.app.PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) android.app.PendingIntent.FLAG_IMMUTABLE else 0)
    )

    val notification = androidx.core.app.NotificationCompat.Builder(context, channelId)
        .setSmallIcon(R.mipmap.ic_launcher)
        .setContentTitle(title)
        .setContentText(message)
        .setStyle(androidx.core.app.NotificationCompat.BigTextStyle().bigText(message))
        .setPriority(androidx.core.app.NotificationCompat.PRIORITY_HIGH)
        .setAutoCancel(true)
        .setContentIntent(pendingIntent)
        .build()

    val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as android.app.NotificationManager
    manager.notify(2000 + termId, notification)
}

class MainActivity : ComponentActivity() {
    private var tunnelClient: TunnelWebSocketClient? = null
    private val pendingDeepLink = mutableStateOf<String?>(null)
    private val pendingTargetTerm = mutableStateOf<Int?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        extractDeepLink(intent)

        setContent {
            OmniLinkModernTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = Color(0xFF090A0F)
                ) {
                    AppScreen()
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        extractDeepLink(intent)
    }

    private fun extractDeepLink(intent: Intent?) {
        val targetTerm = intent?.getIntExtra("EXTRA_TARGET_TERM_ID", -1) ?: -1
        if (targetTerm >= 0) {
            pendingTargetTerm.value = targetTerm
        }
        val uri = intent?.data ?: return
        val raw = uri.toString()
        pendingDeepLink.value = raw
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    fun AppScreen() {
        val context = LocalContext.current
        val coroutineScope = rememberCoroutineScope()
        val prefs = remember { context.getSharedPreferences("omnilink_prefs", Context.MODE_PRIVATE) }

        var savedPairingJson by remember { mutableStateOf(prefs.getString("saved_pairing", null)) }
        var status by remember { mutableStateOf("Ready to connect") }
        var isConnected by remember { mutableStateOf(false) }
        var manualInput by remember { mutableStateOf("") }
        var showScanner by remember { mutableStateOf(false) }

        val tabs = remember { mutableStateListOf(TerminalTab(0, "PowerShell 1")) }
        var activeTabId by remember { mutableStateOf(0) }
        val activeTab = tabs.find { it.id == activeTabId } ?: tabs.firstOrNull() ?: TerminalTab(0, "PowerShell 1")
        var liveInput by remember { mutableStateOf("") }
        val listState = rememberLazyListState()

        val vibrateSuccess = {
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    val vm = context.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as VibratorManager
                    vm.defaultVibrator.vibrate(VibrationEffect.createOneShot(100, VibrationEffect.DEFAULT_AMPLITUDE))
                } else {
                    @Suppress("DEPRECATION")
                    val v = context.getSystemService(Context.VIBRATOR_SERVICE) as Vibrator
                    @Suppress("DEPRECATION")
                    v.vibrate(100)
                }
            } catch (_: Exception) {}
        }

        val connectZeroTrust = { data: PairingData ->
            try {
                vibrateSuccess()
                tunnelClient?.disconnect()
                tabs.forEach { it.screen.clear() }
                status = "Connecting to relay..."

                // Persist pairing so the user NEVER needs to scan again
                val jsonStr = Gson().toJson(data)
                prefs.edit().putString("saved_pairing", jsonStr).apply()
                savedPairingJson = jsonStr

                tunnelClient = TunnelWebSocketClient(
                    relayUrl = data.relay,
                    roomId = data.room,
                    hostPubHex = data.host_pub,
                    authTokenHex = data.auth_token,
                    onOutputReceived = { termId, text ->
                        android.util.Log.i("OmniLink", "onOutputReceived termId=$termId, len=${text.length}")
                        var tab = tabs.find { it.id == termId }
                        if (tab == null) {
                            val newTab = TerminalTab(termId, "Terminal ${termId + 1}")
                            tabs.add(newTab)
                            tab = newTab
                        }
                        tab.screen.feed(text)
                        tab.revision++
                        coroutineScope.launch {
                            if (activeTabId == termId) {
                                val count = tab.screen.renderedLines.size
                                if (count > 0) {
                                    listState.scrollToItem(count - 1)
                                }
                            }
                        }
                    },
                    onStatusChanged = { newStatus ->
                        status = newStatus
                        isConnected = newStatus.contains("Connected") || newStatus.contains("Active")
                    }
                )
                tunnelClient?.connect()
            } catch (e: Exception) {
                status = "Connection error: ${e.message}"
                Toast.makeText(context, "Pairing Error: ${e.message}", Toast.LENGTH_LONG).show()
            }
        }

        val handleRawPayload = { raw: String ->
            val parsed = parsePairingPayload(raw)
            if (parsed != null) {
                connectZeroTrust(parsed)
            } else {
                Toast.makeText(context, "Invalid pairing payload format!", Toast.LENGTH_SHORT).show()
            }
        }

        // Request Notification Permission on Android 13+ (API 33+)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val notifPermissionLauncher = rememberLauncherForActivityResult(
                contract = ActivityResultContracts.RequestPermission()
            ) { _ -> }
            LaunchedEffect(Unit) {
                if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                    notifPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
                }
            }
        }

        // Background Push Notification Subscriber (Listens for AI Agent alerts via Relay)
        LaunchedEffect(savedPairingJson) {
            val saved = savedPairingJson ?: return@LaunchedEffect
            val data = try { Gson().fromJson(saved, PairingData::class.java) } catch (_: Exception) { null } ?: return@LaunchedEffect
            if (data.room.isBlank()) return@LaunchedEffect

            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                val notifWsUrl = data.relay
                    .replace("https://", "wss://")
                    .replace("http://", "ws://") + "/v1/notifications/ws?channel=${data.room}"

                val client = okhttp3.OkHttpClient.Builder()
                    .readTimeout(0, java.util.concurrent.TimeUnit.MILLISECONDS)
                    .pingInterval(15, java.util.concurrent.TimeUnit.SECONDS)
                    .build()

                val request = okhttp3.Request.Builder().url(notifWsUrl).build()

                client.newWebSocket(request, object : okhttp3.WebSocketListener() {
                    override fun onMessage(webSocket: okhttp3.WebSocket, text: String) {
                        try {
                            val notif = Gson().fromJson(text, PushNotificationMsg::class.java)
                            if (notif != null) {
                                val termId = notif.term_id ?: 0
                                showAgentNotification(
                                    context = context,
                                    title = notif.title ?: "OmniLink AI Alert",
                                    message = notif.message ?: "",
                                    termId = termId
                                )
                            }
                        } catch (e: Exception) {
                            android.util.Log.e("OmniLink", "Notification parse error: ${e.message}")
                        }
                    }
                })
            }
        }

        // Direct Deep Link / Notification Click to Target Terminal Session
        LaunchedEffect(pendingTargetTerm.value) {
            val target = pendingTargetTerm.value ?: return@LaunchedEffect
            if (target >= 0) {
                var tab = tabs.find { it.id == target }
                if (tab == null) {
                    val newTab = TerminalTab(target, "Terminal ${target + 1}")
                    tabs.add(newTab)
                }
                activeTabId = target
                pendingTargetTerm.value = null

                if (!isConnected && tunnelClient == null) {
                    val saved = prefs.getString("saved_pairing", null)
                    if (!saved.isNullOrBlank()) {
                        val data = Gson().fromJson(saved, PairingData::class.java)
                        if (data != null && data.room.isNotBlank()) {
                            connectZeroTrust(data)
                        }
                    }
                }
            }
        }

        // Unified Connection Trigger (Handles Deep Link or Saved Pairing without race condition)
        LaunchedEffect(pendingDeepLink.value) {
            val deepLink = pendingDeepLink.value
            if (!deepLink.isNullOrBlank()) {
                handleRawPayload(deepLink)
                pendingDeepLink.value = null
            } else if (!isConnected && tunnelClient == null) {
                val saved = prefs.getString("saved_pairing", null)
                if (!saved.isNullOrBlank()) {
                    val data = Gson().fromJson(saved, PairingData::class.java)
                    if (data != null && data.room.isNotBlank()) {
                        connectZeroTrust(data)
                    }
                }
            }
        }

        val cameraPermissionLauncher = rememberLauncherForActivityResult(
            contract = ActivityResultContracts.RequestPermission()
        ) { isGranted ->
            if (isGranted) {
                showScanner = true
            } else {
                Toast.makeText(context, "Camera permission needed to scan QR code", Toast.LENGTH_LONG).show()
            }
        }

        val openScanner = {
            val hasCamera = ContextCompat.checkSelfPermission(
                context,
                Manifest.permission.CAMERA
            ) == PackageManager.PERMISSION_GRANTED
            if (hasCamera) {
                showScanner = true
            } else {
                cameraPermissionLauncher.launch(Manifest.permission.CAMERA)
            }
        }

        val sendTerminalCommand = { cmd: String ->
            tunnelClient?.sendInput(cmd, activeTabId)
        }

        val spawnTerminalTab = {
            val nextId = (tabs.maxOfOrNull { it.id } ?: 0) + 1
            val newTab = TerminalTab(nextId, "PowerShell ${nextId + 1}")
            tabs.add(newTab)
            activeTabId = nextId
            tunnelClient?.spawnTerminal(nextId)
        }

        val spawnPresetTab = { title: String, cmd: String ->
            val nextId = (tabs.maxOfOrNull { it.id } ?: 0) + 1
            val newTab = TerminalTab(nextId, title)
            tabs.add(newTab)
            activeTabId = nextId
            tunnelClient?.spawnTerminal(nextId)
            coroutineScope.launch {
                kotlinx.coroutines.delay(350)
                tunnelClient?.sendInput("$cmd\r", nextId)
            }
        }

        val closeTerminalTab = { id: Int ->
            if (tabs.size > 1) {
                tunnelClient?.closeTerminal(id)
                val idx = tabs.indexOfFirst { it.id == id }
                if (idx != -1) {
                    tabs.removeAt(idx)
                    if (activeTabId == id) {
                        activeTabId = tabs.firstOrNull()?.id ?: 0
                    }
                }
            }
        }

        val disconnectAll = {
            tunnelClient?.disconnect()
            tunnelClient = null
            isConnected = false
            status = "Disconnected"
        }

        val forgetSavedPairing = {
            disconnectAll()
            prefs.edit().remove("saved_pairing").apply()
            savedPairingJson = null
            status = "Ready to connect"
        }

        // QR Scanner Camera Dialog
        if (showScanner) {
            QrScannerDialog(
                onQrScanned = { qrResult ->
                    showScanner = false
                    handleRawPayload(qrResult)
                },
                onDismiss = {
                    showScanner = false
                }
            )
        }

        Scaffold(
            topBar = {
                Surface(
                    color = Color(0xFF10121A),
                    modifier = Modifier
                        .fillMaxWidth()
                        .statusBarsPadding()
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.SpaceBetween
                    ) {
                        Column {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(
                                    "OMNILINK",
                                    fontSize = 17.sp,
                                    fontWeight = FontWeight.Bold,
                                    color = Color.White
                                )
                                Spacer(modifier = Modifier.width(8.dp))
                                Box(
                                    modifier = Modifier
                                        .size(8.dp)
                                        .clip(CircleShape)
                                        .background(if (isConnected) Color(0xFF10B981) else Color(0xFFEF4444))
                                )
                            }
                            Text(
                                status,
                                fontSize = 11.sp,
                                color = if (isConnected) Color(0xFF10B981) else Color(0xFF94A3B8)
                            )
                        }

                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (savedPairingJson != null && !isConnected) {
                                IconButton(
                                    onClick = { forgetSavedPairing() },
                                    modifier = Modifier.size(32.dp)
                                ) {
                                    Icon(Icons.Default.QrCode, contentDescription = "New Pair", tint = Color(0xFF94A3B8))
                                }
                            }

                            if (isConnected) {
                                Button(
                                    onClick = { disconnectAll() },
                                    colors = ButtonDefaults.buttonColors(containerColor = Color(0xFFEF4444)),
                                    shape = RoundedCornerShape(8.dp),
                                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 4.dp)
                                ) {
                                    Icon(Icons.Default.Stop, contentDescription = "Stop", modifier = Modifier.size(16.dp))
                                    Spacer(modifier = Modifier.width(4.dp))
                                    Text("STOP", fontSize = 12.sp, fontWeight = FontWeight.Bold)
                                }
                            }
                        }
                    }
                }
            }
        ) { padding ->
            if (isConnected) {
                // ==========================================================
                // Active Terminal View
                // ==========================================================
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .imePadding()
                        .background(Color(0xFF090A0F))
                ) {
                    // Multi-Terminal Tab Bar
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(Color(0xFF141724))
                            .horizontalScroll(rememberScrollState())
                            .padding(horizontal = 8.dp, vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        tabs.forEach { tab ->
                            val isSelected = tab.id == activeTabId
                            Surface(
                                onClick = { activeTabId = tab.id },
                                shape = RoundedCornerShape(8.dp),
                                color = if (isSelected) Color(0xFF1E2438) else Color(0xFF0F111A),
                                border = BorderStroke(
                                    1.dp,
                                    if (isSelected) Color(0xFF00F2FE) else Color(0xFF262B3A)
                                ),
                                modifier = Modifier.padding(end = 6.dp)
                            ) {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp)
                                ) {
                                    Icon(
                                        imageVector = Icons.Default.Terminal,
                                        contentDescription = null,
                                        tint = if (isSelected) Color(0xFF00F2FE) else Color(0xFF64748B),
                                        modifier = Modifier.size(14.dp)
                                    )
                                    Spacer(modifier = Modifier.width(6.dp))
                                    Text(
                                        text = tab.title,
                                        fontSize = 12.sp,
                                        fontWeight = if (isSelected) FontWeight.Bold else FontWeight.Normal,
                                        color = if (isSelected) Color.White else Color(0xFF94A3B8)
                                    )
                                    if (tabs.size > 1) {
                                        Spacer(modifier = Modifier.width(6.dp))
                                        IconButton(
                                            onClick = { closeTerminalTab(tab.id) },
                                            modifier = Modifier.size(16.dp)
                                        ) {
                                            Icon(
                                                imageVector = Icons.Default.Close,
                                                contentDescription = "Close Tab",
                                                tint = Color(0xFF94A3B8),
                                                modifier = Modifier.size(12.dp)
                                            )
                                        }
                                    }
                                }
                            }
                        }

                        // Add New Terminal Tab Button [+]
                        IconButton(
                            onClick = { spawnTerminalTab() },
                            modifier = Modifier
                                .size(32.dp)
                                .background(Color(0xFF1A1F30), RoundedCornerShape(8.dp))
                                .border(1.dp, Color(0xFF333B50), RoundedCornerShape(8.dp))
                        ) {
                            Icon(
                                imageVector = Icons.Default.Add,
                                contentDescription = "New Terminal",
                                tint = Color(0xFF00F2FE),
                                modifier = Modifier.size(16.dp)
                            )
                        }

                        Spacer(modifier = Modifier.width(6.dp))

                        // Preset Tab: AGY Auto-Allow
                        Surface(
                            onClick = { spawnPresetTab("AGY 🚀", "agy --dangerously-skip-permissions") },
                            shape = RoundedCornerShape(8.dp),
                            color = Color(0xFF0D2538),
                            border = BorderStroke(1.dp, Color(0xFF00F2FE)),
                            modifier = Modifier.padding(end = 6.dp)
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp)
                            ) {
                                Text(
                                    "+ 🚀 AGY",
                                    fontSize = 11.sp,
                                    fontWeight = FontWeight.Bold,
                                    color = Color(0xFF00F2FE)
                                )
                            }
                        }

                        // Preset Tab: Claude Auto-Allow
                        Surface(
                            onClick = { spawnPresetTab("Claude 🤖", "claude --dangerously-skip-permissions") },
                            shape = RoundedCornerShape(8.dp),
                            color = Color(0xFF2E1C12),
                            border = BorderStroke(1.dp, Color(0xFFF59E0B)),
                            modifier = Modifier.padding(end = 6.dp)
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp)
                            ) {
                                Text(
                                    "+ 🤖 Claude",
                                    fontSize = 11.sp,
                                    fontWeight = FontWeight.Bold,
                                    color = Color(0xFFFBBF24)
                                )
                            }
                        }
                    }

                    LaunchedEffect(activeTabId) {
                        val count = activeTab.screen.renderedLines.size
                        if (count > 0) {
                            listState.scrollToItem(count - 1)
                        }
                    }

                    // Terminal Output Stream with Native ANSI Color & TUI Rendering
                    LazyColumn(
                        state = listState,
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxWidth()
                            .padding(horizontal = 12.dp, vertical = 6.dp)
                    ) {
                        // Observe revision to force recomposition when new lines arrive
                        val _rev = activeTab.revision
                        items(activeTab.screen.renderedLines) { annotatedLine ->
                            Text(
                                text = annotatedLine,
                                color = Color.White,
                                fontFamily = FontFamily.Monospace,
                                fontSize = 12.5.sp,
                                lineHeight = 16.sp
                            )
                        }
                    }

                    // Sleek Floating Keystroke Toolbar
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(Color(0xFF10121A))
                            .horizontalScroll(rememberScrollState())
                            .padding(horizontal = 6.dp, vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        // AI Agent Quick Presets (Execute directly on current terminal)
                        Button(
                            onClick = { sendTerminalCommand("agy --dangerously-skip-permissions\r") },
                            modifier = Modifier
                                .padding(horizontal = 2.dp)
                                .height(32.dp),
                            shape = RoundedCornerShape(6.dp),
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF0D2538)),
                            border = BorderStroke(1.dp, Color(0xFF00F2FE)),
                            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)
                        ) {
                            Text("🚀 AGY (Auto)", fontSize = 11.sp, color = Color(0xFF00F2FE), fontWeight = FontWeight.Bold)
                        }

                        Button(
                            onClick = { sendTerminalCommand("claude --dangerously-skip-permissions\r") },
                            modifier = Modifier
                                .padding(horizontal = 2.dp)
                                .height(32.dp),
                            shape = RoundedCornerShape(6.dp),
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF2E1C12)),
                            border = BorderStroke(1.dp, Color(0xFFF59E0B)),
                            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)
                        ) {
                            Text("🤖 Claude (Auto)", fontSize = 11.sp, color = Color(0xFFFBBF24), fontWeight = FontWeight.Bold)
                        }

                        // High-priority interactive actions: ENTER, Arrows, Space, Esc, ^C
                        Button(
                            onClick = { sendTerminalCommand("\r") },
                            modifier = Modifier
                                .padding(horizontal = 2.dp)
                                .height(32.dp),
                            shape = RoundedCornerShape(6.dp),
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF00F2FE)),
                            contentPadding = PaddingValues(horizontal = 10.dp, vertical = 0.dp)
                        ) {
                            Text("⏎ ENTER", fontSize = 11.sp, color = Color.Black, fontWeight = FontWeight.Bold)
                        }

                        Button(
                            onClick = { sendTerminalCommand("\u001b") },
                            modifier = Modifier
                                .padding(horizontal = 2.dp)
                                .height(32.dp),
                            shape = RoundedCornerShape(6.dp),
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF332010)),
                            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)
                        ) {
                            Text("ESC", fontSize = 11.sp, color = Color(0xFFF59E0B), fontWeight = FontWeight.Bold)
                        }

                        Button(
                            onClick = { sendTerminalCommand("\u0003") },
                            modifier = Modifier
                                .padding(horizontal = 2.dp)
                                .height(32.dp),
                            shape = RoundedCornerShape(6.dp),
                            colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF331515)),
                            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 0.dp)
                        ) {
                            Text("^C", fontSize = 11.sp, color = Color(0xFFEF4444), fontWeight = FontWeight.Bold)
                        }

                        listOf(
                            "▲" to "\u001b[A",
                            "▼" to "\u001b[B",
                            "◀" to "\u001b[D",
                            "▶" to "\u001b[C",
                            "SPACE" to " ",
                            "TAB" to "\t",
                            "y" to "y\r",
                            "n" to "n\r",
                            "^Z" to "\u001a",
                            "^D" to "\u0004",
                            "|" to "|",
                            "~" to "~",
                            "/" to "/"
                        ).forEach { (label, seq) ->
                            Button(
                                onClick = { sendTerminalCommand(seq) },
                                modifier = Modifier
                                    .padding(horizontal = 2.dp)
                                    .height(32.dp),
                                shape = RoundedCornerShape(6.dp),
                                colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF1E2230)),
                                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 0.dp)
                            ) {
                                Text(label, fontSize = 11.sp, color = Color(0xFFE2E8F0), fontWeight = FontWeight.Medium)
                            }
                        }
                    }

                    // Bottom Interactive Command Input Bar (Keystroke Streaming)
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(Color(0xFF0E1017))
                            .padding(8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            "❯",
                            color = Color(0xFF00F2FE),
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier.padding(horizontal = 8.dp)
                        )

                        OutlinedTextField(
                            value = liveInput,
                            onValueChange = { newVal ->
                                if (newVal.length > liveInput.length) {
                                    val delta = newVal.substring(liveInput.length)
                                    tunnelClient?.sendInput(delta, activeTabId)
                                    liveInput = newVal
                                } else if (newVal.length < liveInput.length) {
                                    val backspaces = liveInput.length - newVal.length
                                    repeat(backspaces) {
                                        tunnelClient?.sendInput("\u0008", activeTabId)
                                    }
                                    liveInput = newVal
                                } else {
                                    if (newVal != liveInput) {
                                        repeat(liveInput.length) { tunnelClient?.sendInput("\u0008", activeTabId) }
                                        tunnelClient?.sendInput(newVal, activeTabId)
                                        liveInput = newVal
                                    }
                                }
                            },
                            modifier = Modifier.weight(1f),
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                            keyboardActions = KeyboardActions(onSend = {
                                tunnelClient?.sendInput("\r", activeTabId)
                                liveInput = ""
                            }),
                            placeholder = { Text("Interactive input (streams live)...", color = Color(0xFF475569), fontSize = 13.sp) },
                            singleLine = true,
                            shape = RoundedCornerShape(8.dp),
                            colors = OutlinedTextFieldDefaults.colors(
                                focusedBorderColor = Color(0xFF00F2FE),
                                unfocusedBorderColor = Color(0xFF1E2230),
                                focusedTextColor = Color.White,
                                unfocusedTextColor = Color.White
                            )
                        )

                        Spacer(modifier = Modifier.width(6.dp))

                        IconButton(
                            onClick = {
                                tunnelClient?.sendInput("\r", activeTabId)
                                liveInput = ""
                            },
                            modifier = Modifier
                                .clip(RoundedCornerShape(8.dp))
                                .background(Color(0xFF00F2FE))
                                .size(46.dp)
                        ) {
                            Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send", tint = Color.Black)
                        }
                    }
                }
            } else if (savedPairingJson != null) {
                // ==========================================================
                // Paired Waiting View (Automatic Reconnection Screen)
                // ==========================================================
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .border(1.dp, Color(0xFF262B3A), RoundedCornerShape(20.dp)),
                        colors = CardDefaults.cardColors(containerColor = Color(0xFF12141D)),
                        shape = RoundedCornerShape(20.dp)
                    ) {
                        Column(
                            modifier = Modifier.padding(24.dp),
                            horizontalAlignment = Alignment.CenterHorizontally
                        ) {
                            Box(
                                modifier = Modifier
                                    .size(68.dp)
                                    .clip(CircleShape)
                                    .background(Color(0xFF1E2230)),
                                contentAlignment = Alignment.Center
                            ) {
                                Icon(
                                    Icons.Default.Computer,
                                    contentDescription = "Paired PC",
                                    tint = Color(0xFF00F2FE),
                                    modifier = Modifier.size(36.dp)
                                )
                            }

                            Spacer(modifier = Modifier.height(16.dp))

                            Text(
                                "Paired PC",
                                fontSize = 20.sp,
                                fontWeight = FontWeight.Bold,
                                color = Color.White
                            )

                            Text(
                                status,
                                fontSize = 13.sp,
                                color = if (status.contains("Connecting")) Color(0xFF00F2FE) else Color(0xFF94A3B8),
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                                modifier = Modifier.padding(top = 8.dp, bottom = 24.dp)
                            )

                            Button(
                                onClick = {
                                    val data = Gson().fromJson(savedPairingJson, PairingData::class.java)
                                    if (data != null) connectZeroTrust(data)
                                },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(48.dp),
                                shape = RoundedCornerShape(12.dp),
                                colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF00F2FE))
                            ) {
                                Icon(Icons.Default.Refresh, contentDescription = "Retry", tint = Color.Black)
                                Spacer(modifier = Modifier.width(8.dp))
                                Text("RECONNECT NOW", color = Color.Black, fontWeight = FontWeight.Bold)
                            }

                            Spacer(modifier = Modifier.height(12.dp))

                            OutlinedButton(
                                onClick = { forgetSavedPairing() },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(44.dp),
                                shape = RoundedCornerShape(10.dp),
                                border = androidx.compose.foundation.BorderStroke(1.dp, Color(0xFF262B3A)),
                                colors = ButtonDefaults.outlinedButtonColors(contentColor = Color(0xFF94A3B8))
                            ) {
                                Icon(Icons.Default.QrCodeScanner, contentDescription = "Scan New", modifier = Modifier.size(16.dp))
                                Spacer(modifier = Modifier.width(6.dp))
                                Text("Pair Another PC (Scan QR)", fontSize = 13.sp)
                            }
                        }
                    }
                }
            } else {
                // ==========================================================
                // First-Time Setup & Pairing Screen
                // ==========================================================
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .padding(horizontal = 20.dp, vertical = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .border(1.dp, Color(0xFF262B3A), RoundedCornerShape(20.dp)),
                        colors = CardDefaults.cardColors(containerColor = Color(0xFF12141D)),
                        shape = RoundedCornerShape(20.dp)
                    ) {
                        Column(
                            modifier = Modifier.padding(24.dp),
                            horizontalAlignment = Alignment.CenterHorizontally
                        ) {
                            Box(
                                modifier = Modifier
                                    .size(72.dp)
                                    .clip(RoundedCornerShape(20.dp))
                                    .background(Brush.linearGradient(listOf(Color(0xFF00F2FE), Color(0xFF4FACFE))))
                                    .clickable { openScanner() },
                                contentAlignment = Alignment.Center
                            ) {
                                Icon(
                                    Icons.Default.QrCodeScanner,
                                    contentDescription = "Scan QR",
                                    tint = Color.Black,
                                    modifier = Modifier.size(36.dp)
                                )
                            }

                            Spacer(modifier = Modifier.height(16.dp))

                            Text(
                                "Connect to PC",
                                fontSize = 20.sp,
                                fontWeight = FontWeight.Bold,
                                color = Color.White
                            )

                            Text(
                                "Zero-Trust End-to-End Encrypted Terminal. Tap below to scan the QR code displayed on your PC app.",
                                fontSize = 12.sp,
                                color = Color(0xFF94A3B8),
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                                modifier = Modifier.padding(top = 6.dp, bottom = 20.dp)
                            )

                            Button(
                                onClick = { openScanner() },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(52.dp),
                                shape = RoundedCornerShape(12.dp),
                                colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF00F2FE))
                            ) {
                                Icon(
                                    Icons.Default.CameraAlt,
                                    contentDescription = "Camera",
                                    tint = Color.Black,
                                    modifier = Modifier.size(20.dp)
                                )
                                Spacer(modifier = Modifier.width(8.dp))
                                Text(
                                    "SCAN PC SCREEN QR",
                                    color = Color.Black,
                                    fontWeight = FontWeight.Bold,
                                    fontSize = 14.sp
                                )
                            }

                            Spacer(modifier = Modifier.height(14.dp))

                            OutlinedButton(
                                onClick = {
                                    val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                    val clip = cm.primaryClip
                                    if (clip != null && clip.itemCount > 0) {
                                        val text = clip.getItemAt(0).text?.toString() ?: ""
                                        if (text.isNotBlank()) {
                                            manualInput = text
                                            handleRawPayload(text)
                                        } else {
                                            Toast.makeText(context, "Clipboard is empty", Toast.LENGTH_SHORT).show()
                                        }
                                    } else {
                                        Toast.makeText(context, "Clipboard is empty", Toast.LENGTH_SHORT).show()
                                    }
                                },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(44.dp),
                                shape = RoundedCornerShape(10.dp),
                                border = androidx.compose.foundation.BorderStroke(1.dp, Color(0xFF262B3A)),
                                colors = ButtonDefaults.outlinedButtonColors(contentColor = Color(0xFF94A3B8))
                            ) {
                                Icon(Icons.Default.ContentPaste, contentDescription = "Paste", modifier = Modifier.size(16.dp))
                                Spacer(modifier = Modifier.width(6.dp))
                                Text("Paste from Clipboard", fontSize = 13.sp)
                            }

                            Spacer(modifier = Modifier.height(20.dp))

                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                HorizontalDivider(modifier = Modifier.weight(1f), color = Color(0xFF1E2230))
                                Text(
                                    "  OR PASTE MANUALLY  ",
                                    fontSize = 10.sp,
                                    color = Color(0xFF64748B),
                                    fontWeight = FontWeight.SemiBold
                                )
                                HorizontalDivider(modifier = Modifier.weight(1f), color = Color(0xFF1E2230))
                            }

                            Spacer(modifier = Modifier.height(16.dp))

                            OutlinedTextField(
                                value = manualInput,
                                onValueChange = { manualInput = it },
                                label = { Text("Pairing URL, Base64 or JSON") },
                                placeholder = { Text("https://relay... or {\"relay\":...}") },
                                modifier = Modifier.fillMaxWidth(),
                                singleLine = true,
                                shape = RoundedCornerShape(10.dp),
                                colors = OutlinedTextFieldDefaults.colors(
                                    focusedBorderColor = Color(0xFF00F2FE),
                                    unfocusedBorderColor = Color(0xFF262B3A),
                                    focusedTextColor = Color.White,
                                    unfocusedTextColor = Color.White,
                                    focusedLabelColor = Color(0xFF00F2FE),
                                    unfocusedLabelColor = Color(0xFF64748B)
                                )
                            )

                            Spacer(modifier = Modifier.height(12.dp))

                            Button(
                                onClick = {
                                    if (manualInput.isNotBlank()) {
                                        handleRawPayload(manualInput)
                                    } else {
                                        Toast.makeText(context, "Please scan QR or paste pairing data first", Toast.LENGTH_SHORT).show()
                                    }
                                },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(44.dp),
                                shape = RoundedCornerShape(10.dp),
                                colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF1E293B))
                            ) {
                                Text("Connect With Text", color = Color(0xFFF1F5F9), fontWeight = FontWeight.SemiBold, fontSize = 13.sp)
                            }
                        }
                    }
                }
            }
        }
    }

    @Composable
    fun OmniLinkModernTheme(content: @Composable () -> Unit) {
        MaterialTheme(
            colorScheme = darkColorScheme(
                primary = Color(0xFF00F2FE),
                background = Color(0xFF090A0F),
                surface = Color(0xFF10121A)
            ),
            content = content
        )
    }
}

// ==============================================================================
// Live CameraX QR Code Scanner Dialog
// ==============================================================================

@Composable
fun QrScannerDialog(
    onQrScanned: (String) -> Unit,
    onDismiss: () -> Unit
) {
    val lifecycleOwner = androidx.compose.ui.platform.LocalLifecycleOwner.current
    val cameraExecutor = remember { Executors.newSingleThreadExecutor() }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false)
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(Color.Black)
        ) {
            AndroidView(
                modifier = Modifier.fillMaxSize(),
                factory = { ctx ->
                    val previewView = PreviewView(ctx)
                    val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)

                    cameraProviderFuture.addListener({
                        val cameraProvider = cameraProviderFuture.get()
                        val preview = Preview.Builder().build().also {
                            it.setSurfaceProvider(previewView.surfaceProvider)
                        }

                        val imageAnalysis = ImageAnalysis.Builder()
                            .setTargetResolution(android.util.Size(1280, 720))
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()

                        imageAnalysis.setAnalyzer(
                            cameraExecutor,
                            QrCodeAnalyzer { result ->
                                onQrScanned(result)
                            }
                        )

                        val cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA

                        try {
                            cameraProvider.unbindAll()
                            cameraProvider.bindToLifecycle(
                                lifecycleOwner,
                                cameraSelector,
                                preview,
                                imageAnalysis
                            )
                        } catch (_: Exception) {}
                    }, ContextCompat.getMainExecutor(ctx))

                    previewView
                }
            )

            // Scanning Viewfinder Overlay
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.SpaceBetween
            ) {
                // Top controls
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 16.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        "Align QR Code within frame",
                        color = Color.White,
                        fontSize = 14.sp,
                        fontWeight = FontWeight.Medium
                    )

                    IconButton(
                        onClick = onDismiss,
                        modifier = Modifier
                            .size(36.dp)
                            .clip(CircleShape)
                            .background(Color(0x80000000))
                    ) {
                        Icon(Icons.Default.Close, contentDescription = "Close", tint = Color.White)
                    }
                }

                // Viewfinder Target Box
                Box(
                    modifier = Modifier
                        .size(260.dp)
                        .border(2.dp, Color(0xFF00F2FE), RoundedCornerShape(16.dp))
                )

                // Bottom Hint & Quick Paste
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    modifier = Modifier.padding(bottom = 24.dp)
                ) {
                    Text(
                        "Point at the QR code displayed on your PC app",
                        color = Color(0xFF94A3B8),
                        fontSize = 12.sp
                    )

                    Spacer(modifier = Modifier.height(12.dp))

                    val ctx = androidx.compose.ui.platform.LocalContext.current
                    Button(
                        onClick = {
                            val cm = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            val clip = cm.primaryClip
                            if (clip != null && clip.itemCount > 0) {
                                val text = clip.getItemAt(0).text?.toString() ?: ""
                                if (text.isNotBlank()) {
                                    onQrScanned(text)
                                } else {
                                    Toast.makeText(ctx, "Clipboard is empty", Toast.LENGTH_SHORT).show()
                                }
                            } else {
                                Toast.makeText(ctx, "Clipboard is empty", Toast.LENGTH_SHORT).show()
                            }
                        },
                        colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF1E2230)),
                        shape = RoundedCornerShape(8.dp),
                        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 6.dp)
                    ) {
                        Icon(Icons.Default.ContentPaste, contentDescription = "Paste", tint = Color(0xFF00F2FE), modifier = Modifier.size(16.dp))
                        Spacer(modifier = Modifier.width(6.dp))
                        Text("Paste Copied Link from PC", fontSize = 12.sp, color = Color.White)
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalGetImage::class)
class QrCodeAnalyzer(
    private val onQrDetected: (String) -> Unit
) : ImageAnalysis.Analyzer {
    private val options = BarcodeScannerOptions.Builder()
        .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
        .build()
    private val scanner = BarcodeScanning.getClient(options)
    private var isScanned = false

    override fun analyze(imageProxy: ImageProxy) {
        val mediaImage = imageProxy.image
        if (mediaImage != null && !isScanned) {
            val image = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)
            scanner.process(image)
                .addOnSuccessListener { barcodes ->
                    for (barcode in barcodes) {
                        barcode.rawValue?.let { qrValue ->
                            if (!isScanned) {
                                isScanned = true
                                onQrDetected(qrValue)
                            }
                        }
                    }
                }
                .addOnCompleteListener {
                    imageProxy.close()
                }
        } else {
            imageProxy.close()
        }
    }
}

// ==============================================================================
// Universal Payload Parser (Handles HTTPS URL, Deep Link, Base64, and JSON)
// ==============================================================================

fun decodeBase64Flexible(raw: String): String? {
    val clean = raw.trim()
    val padded = when (clean.length % 4) {
        2 -> "$clean=="
        3 -> "$clean="
        else -> clean
    }

    // 1. Try URL_SAFE
    try {
        val bytes = Base64.decode(padded, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP)
        val s = String(bytes, Charsets.UTF_8)
        if (s.startsWith("{") && s.contains("relay")) return s
    } catch (_: Exception) {}

    // 2. Try DEFAULT
    try {
        val bytes = Base64.decode(padded, Base64.DEFAULT or Base64.NO_PADDING or Base64.NO_WRAP)
        val s = String(bytes, Charsets.UTF_8)
        if (s.startsWith("{") && s.contains("relay")) return s
    } catch (_: Exception) {}

    // 3. Try standard URL_SAFE without extra flags
    try {
        val bytes = Base64.decode(clean, Base64.URL_SAFE)
        val s = String(bytes, Charsets.UTF_8)
        if (s.startsWith("{") && s.contains("relay")) return s
    } catch (_: Exception) {}

    return null
}

fun parsePairingPayload(input: String): PairingData? {
    val trimmed = input.trim()

    // 1. Check if it's a URL (https://relay.findbeatradar.com/pair?data=... or omnilink://pair?data=...)
    if (trimmed.startsWith("http://") || trimmed.startsWith("https://") || trimmed.startsWith("omnilink://")) {
        try {
            val uri = Uri.parse(trimmed)
            val dataParam = uri.getQueryParameter("data")
                ?: uri.getQueryParameter("d")
                ?: uri.fragment
            if (!dataParam.isNullOrBlank()) {
                val decodedJson = decodeBase64Flexible(dataParam)
                if (decodedJson != null) {
                    return Gson().fromJson(decodedJson, PairingData::class.java)
                }
            }
        } catch (_: Exception) {}
    }

    // 2. Check if it's base64 encoded JSON directly
    if (!trimmed.startsWith("{")) {
        val decoded = decodeBase64Flexible(trimmed)
        if (decoded != null) {
            return Gson().fromJson(decoded, PairingData::class.java)
        }
    }

    // 3. Check if it's raw JSON
    try {
        if (trimmed.startsWith("{")) {
            val data = Gson().fromJson(trimmed, PairingData::class.java)
            if (data.relay.isNotBlank() && data.room.isNotBlank()) {
                return data
            }
        }
    } catch (_: Exception) {}

    return null
}
