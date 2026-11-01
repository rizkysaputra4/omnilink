package com.omnilink.android.crypto

import org.bouncycastle.crypto.agreement.X25519Agreement
import org.bouncycastle.crypto.generators.X25519KeyPairGenerator
import org.bouncycastle.crypto.params.KeyParameter
import org.bouncycastle.crypto.params.X25519KeyGenerationParameters
import org.bouncycastle.crypto.params.X25519PrivateKeyParameters
import org.bouncycastle.crypto.params.X25519PublicKeyParameters
import org.bouncycastle.crypto.modes.ChaCha20Poly1305
import org.bouncycastle.crypto.params.AEADParameters
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom

class CryptoEngine {
    private val random = SecureRandom()
    private val privateKey: X25519PrivateKeyParameters
    val publicKeyBytes: ByteArray = ByteArray(32)

    private var cipherSend: ChaCha20Poly1305? = null
    private var cipherRecv: ChaCha20Poly1305? = null
    private var sendKey: ByteArray? = null
    private var recvKey: ByteArray? = null
    private var sendNonce: Long = 0L
    private var recvNonce: Long = 0L

    init {
        val generator = X25519KeyPairGenerator()
        generator.init(X25519KeyGenerationParameters(random))
        val keyPair = generator.generateKeyPair()
        privateKey = keyPair.private as X25519PrivateKeyParameters
        val pub = keyPair.public as X25519PublicKeyParameters
        pub.encode(publicKeyBytes, 0)
    }

    fun completeHandshake(hostPublicKeyBytes: ByteArray) {
        val hostPub = X25519PublicKeyParameters(hostPublicKeyBytes, 0)
        val agreement = X25519Agreement()
        agreement.init(privateKey)

        val sharedSecret = ByteArray(agreement.agreementSize)
        agreement.calculateAgreement(hostPub, sharedSecret, 0)

        // Derive keys matching pctunnel-proto:
        // Host -> Client Key
        val shaHc = MessageDigest.getInstance("SHA-256")
        shaHc.update(sharedSecret)
        shaHc.update("OMNILINK_HOST_TO_CLIENT_V1".toByteArray(Charsets.UTF_8))
        val keyHc = shaHc.digest()

        // Client -> Host Key
        val shaCh = MessageDigest.getInstance("SHA-256")
        shaCh.update(sharedSecret)
        shaCh.update("OMNILINK_CLIENT_TO_HOST_V1".toByteArray(Charsets.UTF_8))
        val keyCh = shaCh.digest()

        // As client: send = keyCh, recv = keyHc
        this.sendKey = keyCh
        this.recvKey = keyHc
        this.cipherSend = ChaCha20Poly1305()
        this.cipherRecv = ChaCha20Poly1305()
        this.sendNonce = 0L
        this.recvNonce = 0L
    }

    private fun makeNonce(counter: Long): ByteArray {
        val nonce = ByteArray(12)
        val buf = ByteBuffer.allocate(8)
        buf.putLong(counter)
        System.arraycopy(buf.array(), 0, nonce, 4, 8)
        return nonce
    }

    @Synchronized
    fun encrypt(plaintext: ByteArray): ByteArray {
        val cipher = cipherSend ?: throw IllegalStateException("Crypto not initialized")
        val key = sendKey ?: throw IllegalStateException("Key not initialized")
        val nonce = makeNonce(sendNonce)

        val params = AEADParameters(KeyParameter(key), 128, nonce)
        cipher.init(true, params)

        val out = ByteArray(8 + cipher.getOutputSize(plaintext.size))
        val nonceBuf = ByteBuffer.allocate(8).putLong(sendNonce)
        System.arraycopy(nonceBuf.array(), 0, out, 0, 8)

        var len = cipher.processBytes(plaintext, 0, plaintext.size, out, 8)
        cipher.doFinal(out, 8 + len)

        sendNonce++
        return out
    }

    @Synchronized
    fun decrypt(ciphertext: ByteArray): ByteArray {
        if (ciphertext.size < 8) throw IllegalArgumentException("Ciphertext too short")
        val cipher = cipherRecv ?: throw IllegalStateException("Crypto not initialized")
        val key = recvKey ?: throw IllegalStateException("Key not initialized")

        val nonceBuf = ByteBuffer.wrap(ciphertext, 0, 8)
        val counter = nonceBuf.long
        if (counter < recvNonce) {
            throw SecurityException("Replay attack detected: nonce $counter < expected $recvNonce")
        }
        recvNonce = counter + 1

        val nonce = makeNonce(counter)
        val params = AEADParameters(KeyParameter(key), 128, nonce)
        cipher.init(false, params)

        val payloadLen = ciphertext.size - 8
        val out = ByteArray(cipher.getOutputSize(payloadLen))
        var len = cipher.processBytes(ciphertext, 8, payloadLen, out, 0)
        len += cipher.doFinal(out, len)

        return out.copyOf(len)
    }
}
