package com.pctunnel.android

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import com.google.gson.Gson
import com.pctunnel.android.network.TunnelWebSocketClient
import kotlinx.coroutines.launch

data class PairingData(
    val relay: String,
    val room: String,
    val host_pub: String,
    val pin: String
)

class MainActivity : ComponentActivity() {
    private var tunnelClient: TunnelWebSocketClient? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        setContent {
            PCTunnelTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    AppScreen()
                }
            }
        }
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    fun AppScreen() {
        val context = LocalContext.current
        val coroutineScope = rememberCoroutineScope()
        var status by remember { mutableStateOf("Not connected") }
        var isConnected by remember { mutableStateOf(false) }
        var manualJsonInput by remember { mutableStateOf("") }
        val terminalOutput = remember { mutableStateListOf<String>() }
        var currentInput by remember { mutableStateOf("") }
        val listState = rememberLazyListState()

        val connectToPayload = { payloadJson: String ->
            try {
                val data = Gson().fromJson(payloadJson, PairingData::class.java)
                tunnelClient?.disconnect()
                tunnelClient = TunnelWebSocketClient(
                    relayUrl = data.relay,
                    roomId = data.room,
                    hostPubHex = data.host_pub,
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
                Toast.makeText(context, "Invalid Pairing Data: ${e.message}", Toast.LENGTH_SHORT).show()
            }
        }

        Scaffold(
            topBar = {
                TopAppBar(
                    title = {
                        Column {
                            Text("PCTunnel Mobile", fontSize = 18.sp)
                            Text(status, fontSize = 12.sp, color = if (isConnected) Color.Green else Color.LightGray)
                        }
                    },
                    actions = {
                        if (isConnected) {
                            IconButton(onClick = {
                                tunnelClient?.disconnect()
                                isConnected = false
                            }) {
                                Icon(Icons.Default.Stop, contentDescription = "Disconnect", tint = Color.Red)
                            }
                        }
                    },
                    colors = TopAppBarDefaults.topAppBarColors(containerColor = Color(0xFF1E1E1E), titleContentColor = Color.White)
                )
            }
        ) { padding ->
            if (!isConnected) {
                // Setup / Pairing Screen
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .padding(24.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    Icon(
                        Icons.Default.QrCodeScanner,
                        contentDescription = "Scan QR",
                        modifier = Modifier.size(96.dp),
                        tint = MaterialTheme.colorScheme.primary
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Text("Pair With Your PC", fontSize = 22.sp)
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        "Scan the QR code displayed on your PC screen to establish zero-trust terminal access in 3 seconds.",
                        color = Color.Gray,
                        textAlign = androidx.compose.ui.text.style.TextAlign.Center
                    )
                    Spacer(modifier = Modifier.height(24.dp))

                    OutlinedTextField(
                        value = manualJsonInput,
                        onValueChange = { manualJsonInput = it },
                        label = { Text("Or Paste Pairing JSON Payload") },
                        modifier = Modifier.fillMaxWidth()
                    )
                    Spacer(modifier = Modifier.height(16.dp))

                    Button(
                        onClick = { connectToPayload(manualJsonInput) },
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Icon(Icons.Default.Link, contentDescription = null)
                        Spacer(modifier = Modifier.width(8.dp))
                        Text("Connect Now")
                    }
                }
            } else {
                // Live Interactive Terminal Screen
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .background(Color.Black)
                ) {
                    // Output Display
                    LazyColumn(
                        state = listState,
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxWidth()
                            .padding(8.dp)
                    ) {
                        items(terminalOutput) { chunk ->
                            Text(
                                text = chunk,
                                color = Color(0xFF00FF66),
                                fontFamily = FontFamily.Monospace,
                                fontSize = 13.sp
                            )
                        }
                    }

                    // Mobile Keystroke Bar
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(Color(0xFF252526))
                            .horizontalScroll(rememberScrollState())
                            .padding(horizontal = 4.dp, vertical = 2.dp)
                    ) {
                        listOf(
                            "Esc" to "\u001b",
                            "Tab" to "\t",
                            "Ctrl+C" to "\u0003",
                            "Ctrl+Z" to "\u001a",
                            "Ctrl+D" to "\u0004",
                            "▲" to "\u001b[A",
                            "▼" to "\u001b[B",
                            "◀" to "\u001b[D",
                            "▶" to "\u001b[C"
                        ).forEach { (label, seq) ->
                            Button(
                                onClick = { tunnelClient?.sendInput(seq) },
                                modifier = Modifier.padding(horizontal = 2.dp),
                                colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF3E3E42)),
                                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp)
                            ) {
                                Text(label, fontSize = 11.sp, color = Color.White)
                            }
                        }
                    }

                    // Command Input Line
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(Color(0xFF1E1E1E))
                            .padding(8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(">", color = Color(0xFF00FF66), modifier = Modifier.padding(end = 8.dp))
                        OutlinedTextField(
                            value = currentInput,
                            onValueChange = { currentInput = it },
                            modifier = Modifier.weight(1f),
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                            keyboardActions = KeyboardActions(onSend = {
                                tunnelClient?.sendInput(currentInput + "\r\n")
                                currentInput = ""
                            }),
                            placeholder = { Text("Type command...", color = Color.Gray) },
                            singleLine = true
                        )
                        IconButton(onClick = {
                            tunnelClient?.sendInput(currentInput + "\r\n")
                            currentInput = ""
                        }) {
                            Icon(Icons.Default.Send, contentDescription = "Send", tint = MaterialTheme.colorScheme.primary)
                        }
                    }
                }
            }
        }
    }

    @Composable
    fun PCTunnelTheme(content: @Composable () -> Unit) {
        MaterialTheme(
            colorScheme = darkColorScheme(
                primary = Color(0xFF007ACC),
                background = Color(0xFF121212),
                surface = Color(0xFF1E1E1E)
            ),
            content = content
        )
    }
}
