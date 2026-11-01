package com.omnilink.android

import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import com.google.gson.Gson
import com.omnilink.android.network.TunnelWebSocketClient
import kotlinx.coroutines.launch

data class PairingData(
    val relay: String,
    val room: String,
    val host_pub: String,
    val auth_token: String,
    val pin: String
)

class MainActivity : ComponentActivity() {
    private var tunnelClient: TunnelWebSocketClient? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    fun AppScreen() {
        val context = LocalContext.current
        val coroutineScope = rememberCoroutineScope()

        var status by remember { mutableStateOf("Ready to connect") }
        var isConnected by remember { mutableStateOf(false) }
        var manualJsonInput by remember { mutableStateOf("") }

        val terminalOutput = remember { mutableStateListOf<String>() }
        var currentInput by remember { mutableStateOf("") }
        val listState = rememberLazyListState()

        val connectZeroTrust = { payloadJson: String ->
            try {
                val data = Gson().fromJson(payloadJson, PairingData::class.java)
                tunnelClient?.disconnect()
                terminalOutput.clear()
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
                Toast.makeText(context, "Invalid Pairing Data: ${e.message}", Toast.LENGTH_SHORT).show()
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
                                        .size(7.dp)
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
        ) { padding ->
            if (!isConnected) {
                // Modern Setup & Pairing Screen
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding)
                        .padding(20.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .border(1.dp, Color(0xFF262B3A), RoundedCornerShape(16.dp)),
                        colors = CardDefaults.cardColors(containerColor = Color(0xFF12141D)),
                        shape = RoundedCornerShape(16.dp)
                    ) {
                        Column(
                            modifier = Modifier.padding(22.dp),
                            horizontalAlignment = Alignment.CenterHorizontally
                        ) {
                            Box(
                                modifier = Modifier
                                    .size(64.dp)
                                    .clip(RoundedCornerShape(16.dp))
                                    .background(Brush.linearGradient(listOf(Color(0xFF00F2FE), Color(0xFF4FACFE)))),
                                contentAlignment = Alignment.Center
                            ) {
                                Icon(
                                    Icons.Default.QrCodeScanner,
                                    contentDescription = "Scan",
                                    tint = Color.Black,
                                    modifier = Modifier.size(32.dp)
                                )
                            }

                            Spacer(modifier = Modifier.height(16.dp))
                            Text(
                                "Scan PC Screen QR",
                                fontSize = 20.sp,
                                fontWeight = FontWeight.Bold,
                                color = Color.White
                            )
                            Text(
                                "Zero-Trust End-to-End Encrypted Relay Connection (X25519 + ChaCha20-Poly1305).",
                                fontSize = 12.sp,
                                color = Color(0xFF94A3B8),
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                                modifier = Modifier.padding(top = 6.dp, bottom = 20.dp)
                            )

                            OutlinedTextField(
                                value = manualJsonInput,
                                onValueChange = { manualJsonInput = it },
                                label = { Text("Paste Pairing Payload JSON") },
                                modifier = Modifier.fillMaxWidth(),
                                shape = RoundedCornerShape(10.dp),
                                colors = OutlinedTextFieldDefaults.colors(
                                    focusedBorderColor = Color(0xFF00F2FE),
                                    unfocusedBorderColor = Color(0xFF262B3A),
                                    focusedTextColor = Color.White,
                                    unfocusedTextColor = Color.White
                                )
                            )

                            Spacer(modifier = Modifier.height(16.dp))

                            Button(
                                onClick = { connectZeroTrust(manualJsonInput) },
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .height(48.dp),
                                shape = RoundedCornerShape(10.dp),
                                colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF00F2FE))
                            ) {
                                Text("Pair & Connect", color = Color.Black, fontWeight = FontWeight.Bold)
                            }
                        }
                    }
                }
            } else {
                // Modern Minimalist Terminal View
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
                            placeholder = { Text("Enter shell command...", color = Color(0xFF475569), fontSize = 13.sp) },
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
                            Icon(Icons.Default.Send, contentDescription = "Send", tint = Color.Black)
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
