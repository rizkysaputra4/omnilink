package com.omnilink.android.network

import android.os.Build
import com.omnilink.android.crypto.CryptoEngine
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import okhttp3.*
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.nio.ByteBuffer
import java.util.concurrent.TimeUnit
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

class TunnelWebSocketClient(
    private val relayUrl: String,
    private val roomId: String,
    private val hostPubHex: String,
    private val authTokenHex: String,
    private val onOutputReceived: (termId: Int, text: String) -> Unit,
    private val onStatusChanged: (String) -> Unit
) {
    private val client = OkHttpClient.Builder()
        .pingInterval(15, TimeUnit.SECONDS)
        .build()

    private var webSocket: WebSocket? = null
    private val crypto = CryptoEngine()
    private var isPaired = false
    private var isManualDisconnect = false
    private val clientScope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.IO + kotlinx.coroutines.SupervisorJob())

    fun connect() {
        isManualDisconnect = false
        onStatusChanged("Connecting to relay...")
        val base = relayUrl.trimEnd('/')
        val endpoint = if (base.endsWith("/v1/tunnel")) base else "$base/v1/tunnel"
        val url = "$endpoint?room=$roomId&role=client"
        val request = Request.Builder().url(url).build()

        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(ws: WebSocket, response: Response) {
                android.util.Log.i("OmniLink", "WebSocket onOpen")
                onStatusChanged("Relay connected. Waiting for PC pairing...")
            }

            override fun onMessage(ws: WebSocket, text: String) {
                android.util.Log.i("OmniLink", "WebSocket onMessage text: $text")
                if (text == "OMNILINK_PEER_CONNECTED") {
                    onStatusChanged("Host PC connected. Performing E2EE handshake & Proof...")
                    sendHandshakeInit(ws)
                }
            }

            override fun onMessage(ws: WebSocket, bytes: ByteString) {
                android.util.Log.i("OmniLink", "WebSocket onMessage bytes: size=${bytes.size}")
                handleIncomingBytes(bytes.toByteArray())
            }

            override fun onClosed(ws: WebSocket, code: Int, reason: String) {
                android.util.Log.i("OmniLink", "WebSocket onClosed: $code $reason")
                isPaired = false
                if (!isManualDisconnect) {
                    onStatusChanged("Disconnected. Reconnecting...")
                    clientScope.launch {
                        delay(2000)
                        if (!isManualDisconnect) connect()
                    }
                } else {
                    onStatusChanged("Disconnected")
                }
            }

            override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) {
                android.util.Log.e("OmniLink", "WebSocket onFailure: ${t.message}", t)
                isPaired = false
                if (!isManualDisconnect) {
                    onStatusChanged("Connection lost. Reconnecting...")
                    clientScope.launch {
                        delay(2000)
                        if (!isManualDisconnect) connect()
                    }
                } else {
                    onStatusChanged("Connection error: ${t.message}")
                }
            }
        })
    }

    private fun sendHandshakeInit(ws: WebSocket) {
        val deviceName = "${Build.MANUFACTURER} ${Build.MODEL}".toByteArray(Charsets.UTF_8)
        val authTokenBytes = hexStringToByteArray(authTokenHex)

        // Compute HMAC-SHA256 proof of possession of the QR secret
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(authTokenBytes, "HmacSHA256"))
        mac.update(crypto.publicKeyBytes)
        mac.update(deviceName)
        val authProof = mac.doFinal()

        // Payload format: [pub (32)][proof (32)][id_len (2)][id (N)]
        val payload = ByteBuffer.allocate(32 + 32 + 2 + deviceName.size)
        payload.put(crypto.publicKeyBytes)
        payload.put(authProof)
        payload.putShort(deviceName.size.toShort())
        payload.put(deviceName)

        val frame = encodeFrame(0x01, payload.array())
        ws.send(frame.toByteString())
    }

    private fun handleIncomingBytes(raw: ByteArray) {
        if (!isPaired) {
            // Handshake Response is [type: 0x02][len: 4][hostPub: 32][status: 1]
            if (raw.size >= 5 && raw[0] == 0x02.toByte()) {
                val status = raw[raw.size - 1].toInt()
                if (status != 0) {
                    onStatusChanged("🛑 Authentication Failed! PC rejected connection.")
                    disconnect()
                    return
                }

                val hostPub = ByteArray(32)
                System.arraycopy(raw, 5, hostPub, 0, 32)
                crypto.completeHandshake(hostPub)
                isPaired = true
                android.util.Log.i("OmniLink", "Handshake completed! isPaired=true")
                onStatusChanged("🔒 Connected & Encrypted (E2EE Active)")
            }
        } else {
            // Decrypt payload
            try {
                val plaintext = crypto.decrypt(raw)
                android.util.Log.i("OmniLink", "Decrypted frame, size=${plaintext.size}")
                if (plaintext.size >= 5) {
                    val frameType = plaintext[0].toInt()
                    val len = ByteBuffer.wrap(plaintext, 1, 4).int
                    val payload = ByteArray(len)
                    System.arraycopy(plaintext, 5, payload, 0, len)

                    android.util.Log.i("OmniLink", "FrameType=0x${Integer.toHexString(frameType)}, payloadLen=$len")
                    when (frameType) {
                        0x04 -> { // TerminalOutput
                            if (payload.size >= 4) {
                                val termId = ByteBuffer.wrap(payload, 0, 4).int
                                val text = String(payload, 4, payload.size - 4, Charsets.UTF_8)
                                android.util.Log.i("OmniLink", "TerminalOutput for termId=$termId, len=${text.length}: ${text.take(40)}")
                                onOutputReceived(termId, text)
                            } else {
                                val text = String(payload, Charsets.UTF_8)
                                android.util.Log.i("OmniLink", "TerminalOutput legacy, len=${text.length}")
                                onOutputReceived(0, text)
                            }
                        }
                        0x06 -> { // Ping
                            val pong = encodeFrame(0x07, payload)
                            sendEncrypted(pong)
                        }
                    }
                }
            } catch (e: Exception) {
                android.util.Log.e("OmniLink", "Decryption error: ${e.message}", e)
                onStatusChanged("Decryption error: ${e.message}")
            }
        }
    }

    fun sendInput(input: String, termId: Int = 0) {
        if (!isPaired) {
            android.util.Log.w("OmniLink", "sendInput called but !isPaired")
            return
        }
        val textBytes = input.toByteArray(Charsets.UTF_8)
        val buf = ByteBuffer.allocate(4 + textBytes.size)
        buf.putInt(termId)
        buf.put(textBytes)
        val frame = encodeFrame(0x03, buf.array()) // TerminalInput
        android.util.Log.i("OmniLink", "sendInput termId=$termId, chars=${textBytes.size}")
        sendEncrypted(frame)
    }

    fun spawnTerminal(termId: Int, shell: String = "") {
        if (!isPaired) return
        val sBytes = shell.toByteArray(Charsets.UTF_8)
        val buf = ByteBuffer.allocate(4 + 2 + sBytes.size)
        buf.putInt(termId)
        buf.putShort(sBytes.size.toShort())
        buf.put(sBytes)
        val frame = encodeFrame(0x09, buf.array()) // SpawnTerminal
        sendEncrypted(frame)
    }

    fun closeTerminal(termId: Int) {
        if (!isPaired) return
        val buf = ByteBuffer.allocate(4)
        buf.putInt(termId)
        val frame = encodeFrame(0x0A, buf.array()) // CloseTerminal
        sendEncrypted(frame)
    }

    fun sendResize(cols: Int, rows: Int, termId: Int = 0) {
        if (!isPaired) return
        val buf = ByteBuffer.allocate(4 + 2 + 2)
        buf.putInt(termId)
        buf.putShort(cols.toShort())
        buf.putShort(rows.toShort())
        val frame = encodeFrame(0x05, buf.array()) // Resize
        sendEncrypted(frame)
    }

    private fun sendEncrypted(frame: ByteArray) {
        val ws = webSocket ?: return
        try {
            val encrypted = crypto.encrypt(frame)
            ws.send(encrypted.toByteString())
        } catch (e: Exception) {
            // Error handling
        }
    }

    private fun encodeFrame(type: Int, payload: ByteArray): ByteArray {
        val out = ByteBuffer.allocate(1 + 4 + payload.size)
        out.put(type.toByte())
        out.putInt(payload.size)
        out.put(payload)
        return out.array()
    }

    fun disconnect() {
        isManualDisconnect = true
        val frame = encodeFrame(0x08, "User closed session".toByteArray(Charsets.UTF_8))
        try {
            sendEncrypted(frame)
        } catch (_: Exception) {}
        webSocket?.close(1000, "Disconnect requested")
        webSocket = null
        isPaired = false
        onStatusChanged("Disconnected")
    }

    private fun hexStringToByteArray(s: String): ByteArray {
        val len = s.length
        val data = ByteArray(len / 2)
        var i = 0
        while (i < len) {
            data[i / 2] = ((Character.digit(s[i], 16) shl 4) + Character.digit(s[i + 1], 16)).toByte()
            i += 2
        }
        return data
    }
}
