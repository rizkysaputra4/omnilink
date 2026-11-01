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
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.common.InputImage
import com.omnilink.android.network.TunnelWebSocketClient
import kotlinx.coroutines.launch
import java.util.concurrent.Executors

data class PairingData(
    val relay: String,
    val room: String,
    val host_pub: String,
    val auth_token: String,
    val pin: String
)

class MainActivity : ComponentActivity() {
    private var tunnelClient: TunnelWebSocketClient? = null
    private val pendingDeepLink = mutableStateOf<String?>(null)

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

        val terminalOutput = remember { mutableStateListOf<String>() }
        var currentInput by remember { mutableStateOf("") }
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
                terminalOutput.clear()
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
                    onOutputReceived = { text ->
                        terminalOutput.add(text)
                        coroutineScope.launch {
                            if (terminalOutput.isNotEmpty()) {
                                listState.animateScrollToItem(terminalOutput.size - 1)
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

        // Auto-connect on app launch if already paired!
        LaunchedEffect(Unit) {
            val saved = prefs.getString("saved_pairing", null)
            if (!saved.isNullOrBlank()) {
                val data = Gson().fromJson(saved, PairingData::class.java)
                if (data != null && data.room.isNotBlank()) {
                    connectZeroTrust(data)
                }
            }
        }

        // Trigger auto-connect if launched via deep link
        LaunchedEffect(pendingDeepLink.value) {
            val deepLink = pendingDeepLink.value
            if (!deepLink.isNullOrBlank()) {
                handleRawPayload(deepLink)
                pendingDeepLink.value = null
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
            tunnelClient?.sendInput(cmd)
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
                    modifier = Modifier.fillMaxWidth()
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
                        .background(Color(0xFF090A0F))
                ) {
                    // Terminal Output Stream
                    LazyColumn(
                        state = listState,
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxWidth()
                            .padding(horizontal = 12.dp, vertical = 6.dp)
                    ) {
                        items(terminalOutput) { chunk ->
                            Text(
                                text = chunk,
                                color = Color(0xFF38BDF8),
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
                        listOf(
                            "ESC" to "\u001b",
                            "TAB" to "\t",
                            "^C" to "\u0003",
                            "^Z" to "\u001a",
                            "^D" to "\u0004",
                            "▲" to "\u001b[A",
                            "▼" to "\u001b[B",
                            "◀" to "\u001b[D",
                            "▶" to "\u001b[C",
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

                    // Bottom Command Input Bar
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
                            value = currentInput,
                            onValueChange = { currentInput = it },
                            modifier = Modifier.weight(1f),
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                            keyboardActions = KeyboardActions(onSend = {
                                sendTerminalCommand(currentInput + "\r\n")
                                currentInput = ""
                            }),
                            placeholder = { Text("Enter command...", color = Color(0xFF475569), fontSize = 13.sp) },
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
                                sendTerminalCommand(currentInput + "\r\n")
                                currentInput = ""
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

                // Bottom Hint
                Text(
                    "Point at the QR code displayed on your PC app",
                    color = Color(0xFF94A3B8),
                    fontSize = 12.sp,
                    modifier = Modifier.padding(bottom = 32.dp)
                )
            }
        }
    }
}

@OptIn(ExperimentalGetImage::class)
class QrCodeAnalyzer(
    private val onQrDetected: (String) -> Unit
) : ImageAnalysis.Analyzer {
    private val scanner = BarcodeScanning.getClient()
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
                val decodedJson = String(
                    Base64.decode(dataParam, Base64.URL_SAFE or Base64.DEFAULT),
                    Charsets.UTF_8
                )
                return Gson().fromJson(decodedJson, PairingData::class.java)
            }
        } catch (_: Exception) {}
    }

    // 2. Check if it's base64 encoded JSON directly
    if (!trimmed.startsWith("{")) {
        try {
            val decoded = String(
                Base64.decode(trimmed, Base64.URL_SAFE or Base64.DEFAULT),
                Charsets.UTF_8
            )
            if (decoded.startsWith("{") && decoded.contains("relay")) {
                return Gson().fromJson(decoded, PairingData::class.java)
            }
        } catch (_: Exception) {}
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
