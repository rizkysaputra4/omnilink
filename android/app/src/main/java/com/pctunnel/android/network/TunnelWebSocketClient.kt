package com.pctunnel.android.network

import android.os.Build
import com.pctunnel.android.crypto.CryptoEngine
import okhttp3.*
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.nio.ByteBuffer
import java.util.concurrent.TimeUnit

class TunnelWebSocketClient(
    private val relayUrl: String,
    private val roomId: String,
    private val hostPubHex: String,
    private val onOutputReceived: (String) -> Unit,
    private val onStatusChanged: (String) -> Unit
) {
    private val client = OkHttpClient.Builder()
        .pingInterval(15, TimeUnit.SECONDS)
        .build()

    private var webSocket: WebSocket? = null
    private val crypto = CryptoEngine()
    private var isPaired = false

    fun connect() {
        onStatusChanged("Connecting to relay...")
        val url = "$relayUrl/v1/tunnel?room=$roomId&role=client"
        val request = Request.Builder().url(url).build()

        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(ws: WebSocket, response: Response) {
                onStatusChanged("Relay connected. Waiting for PC pairing...")
            }

            override fun onMessage(ws: WebSocket, text: String) {
                if (text == "PCTUNNEL_PEER_CONNECTED") {
                    onStatusChanged("Host PC connected. Performing E2EE handshake...")
                    sendHandshakeInit(ws)
                }
            }

            override fun onMessage(ws: WebSocket, bytes: ByteString) {
                handleIncomingBytes(bytes.toByteArray())
            }

            override fun onClosed(ws: WebSocket, code: Int, reason: String) {
                onStatusChanged("Connection closed: $reason")
            }

            override fun onFailure(ws: WebSocket, t: Throwable, response: Response?) {
                onStatusChanged("Connection error: ${t.message}")
            }
        })
    }

    private fun sendHandshakeInit(ws: WebSocket) {
        val deviceName = "${Build.MANUFACTURER} ${Build.MODEL}".toByteArray(Charsets.UTF_8)
        val payload = ByteBuffer.allocate(32 + 2 + deviceName.size)
        payload.put(crypto.publicKeyBytes)
        payload.putShort(deviceName.size.toShort())
        payload.put(deviceName)

        val frame = encodeFrame(0x01, payload.array())
        ws.send(frame.toByteString())
    }

    private fun handleIncomingBytes(raw: ByteArray) {
        if (!isPaired) {
            // Handshake Response is unencrypted
            if (raw.size >= 5 && raw[0] == 0x02.toByte()) {
                val len = ByteBuffer.wrap(raw, 1, 4).int
                val hostPub = ByteArray(32)
                System.arraycopy(raw, 5, hostPub, 0, 32)
                crypto.completeHandshake(hostPub)
                isPaired = true
                onStatusChanged("🔒 Connected & Encrypted (E2EE Active)")
            }
        } else {
            // Decrypt payload
            try {
                val plaintext = crypto.decrypt(raw)
                if (plaintext.size >= 5) {
                    val frameType = plaintext[0].toInt()
                    val len = ByteBuffer.wrap(plaintext, 1, 4).int
                    val payload = ByteArray(len)
                    System.arraycopy(plaintext, 5, payload, 0, len)

                    when (frameType) {
                        0x04 -> { // TerminalOutput
                            val text = String(payload, Charsets.UTF_8)
                            onOutputReceived(text)
                        }
                        0x06 -> { // Ping
                            val pong = encodeFrame(0x07, payload)
                            sendEncrypted(pong)
                        }
                    }
                }
            } catch (e: Exception) {
                onStatusChanged("Decryption error: ${e.message}")
            }
        }
    }

    fun sendInput(input: String) {
        if (!isPaired) return
        val data = input.toByteArray(Charsets.UTF_8)
        val frame = encodeFrame(0x03, data) // TerminalInput
        sendEncrypted(frame)
    }

    fun sendResize(cols: Int, rows: Int) {
        if (!isPaired) return
        val buf = ByteBuffer.allocate(4)
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
            // Log or notify
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
        val frame = encodeFrame(0x08, "User closed session".toByteArray(Charsets.UTF_8))
        try {
            sendEncrypted(frame)
        } catch (_: Exception) {}
        webSocket?.close(1000, "Disconnect requested")
        webSocket = null
        onStatusChanged("Disconnected")
    }
}
