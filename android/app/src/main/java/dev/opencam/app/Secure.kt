package dev.opencam.app

import java.math.BigInteger
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.security.KeyFactory
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.interfaces.ECPrivateKey
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECParameterSpec
import java.security.spec.ECPoint
import java.security.spec.ECPrivateKeySpec
import java.security.spec.ECPublicKeySpec
import java.security.spec.ECFieldFp
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Emparejamiento y cifrado OPCM (docs/PROTOCOL.md, «Seguridad»), igual que `core/src/secure.rs`:
 * ECDH P-256 → HKDF-SHA256 (sal = PSK) → AUTH con HMAC → AES-256-GCM con la cabecera como AAD.
 * Solo usa la JCA estándar, así que se prueba en la JVM.
 */
object Secure {
    const val TAG_LEN = 16
    const val PUBLIC_KEY_LEN = 65
    const val MAX_UNAUTH_PAYLOAD = 64 * 1024
    const val MAX_CODE_ATTEMPTS = 5

    private val INFO = "opencam-v1".toByteArray()
    private val CODE_SALT = "opencam-pair-code:".toByteArray()
    private val random = SecureRandom()

    class AuthFailed : Exception("autenticación fallida")

    /** Normaliza «123 456» o «123-456»; null si no son 6 dígitos. */
    fun normalizeCode(code: String): String? {
        val d = code.filterNot { it.isWhitespace() || it == '-' }
        return d.takeIf { it.length == 6 && it.all { c -> c in '0'..'9' } }
    }

    fun codePsk(code: String): ByteArray {
        val c = requireNotNull(normalizeCode(code)) { "código inválido" }
        return sha256(CODE_SALT + c.toByteArray())
    }

    /** Código de 6 dígitos uniforme. */
    fun newCode(rng: SecureRandom = random): String = "%06d".format(rng.nextInt(1_000_000))

    fun randomBytes(n: Int): ByteArray = ByteArray(n).also { random.nextBytes(it) }

    fun generateKeyPair(): KeyPair =
        KeyPairGenerator.getInstance("EC").apply { initialize(ECGenParameterSpec("secp256r1")) }.generateKeyPair()

    /** Solo para vectores de prueba: par de claves a partir del escalar privado. */
    fun keyPairFromPrivate(d: ByteArray): KeyPair {
        val params = (generateKeyPair().public as ECPublicKey).params
        val kf = KeyFactory.getInstance("EC")
        val s = BigInteger(1, d)
        val priv = kf.generatePrivate(ECPrivateKeySpec(s, params))
        val pub = kf.generatePublic(ECPublicKeySpec(multiplyG(s, params), params))
        return KeyPair(pub, priv)
    }

    /** Punto sin comprimir: 0x04 || X || Y (32 bytes cada uno). */
    fun publicBytes(kp: KeyPair): ByteArray {
        val w = (kp.public as ECPublicKey).w
        return byteArrayOf(4) + fixed32(w.affineX) + fixed32(w.affineY)
    }

    /** Decodifica y valida (dentro de la curva) la clave pública del otro lado. */
    fun decodePublic(bytes: ByteArray, params: ECParameterSpec): ECPublicKey {
        require(bytes.size == PUBLIC_KEY_LEN && bytes[0] == 4.toByte()) { "clave pública inválida" }
        val x = BigInteger(1, bytes.copyOfRange(1, 33))
        val y = BigInteger(1, bytes.copyOfRange(33, 65))
        val p = (params.curve.field as ECFieldFp).p
        require(x < p && y < p) { "clave pública inválida" }
        // y² = x³ + ax + b (mod p): rechaza puntos fuera de la curva (ataque de curva inválida).
        val lhs = y.modPow(BigInteger.valueOf(2), p)
        val rhs = x.pow(3).add(params.curve.a.multiply(x)).add(params.curve.b).mod(p)
        require(lhs == rhs) { "clave pública inválida" }
        return KeyFactory.getInstance("EC").generatePublic(ECPublicKeySpec(ECPoint(x, y), params)) as ECPublicKey
    }

    fun transcriptHash(desktopHello: ByteArray, phoneHello: ByteArray): ByteArray =
        sha256(be32(desktopHello.size) + desktopHello + be32(phoneHello.size) + phoneHello)

    class SessionKeys(
        private val desktopToPhone: ByteArray,
        private val phoneToDesktop: ByteArray,
        private val confirm: ByteArray,
        val pairingKey: ByteArray,
    ) {
        fun authTag(role: Role): ByteArray = hmac(confirm, role.label)

        fun verifyAuth(role: Role, tag: ByteArray) {
            if (!MessageDigest.isEqual(authTag(role), tag)) throw AuthFailed()
        }

        /** (envío, recepción) para `role`. */
        fun ciphers(role: Role): Pair<GcmCipher, GcmCipher> {
            val d2p = GcmCipher(desktopToPhone)
            val p2d = GcmCipher(phoneToDesktop)
            return if (role == Role.DESKTOP) d2p to p2d else p2d to d2p
        }
    }

    enum class Role(val label: ByteArray) {
        DESKTOP("desktop".toByteArray()),
        PHONE("phone".toByteArray()),
    }

    fun derive(own: KeyPair, peerPublic: ByteArray, psk: ByteArray, desktopHello: ByteArray, phoneHello: ByteArray): SessionKeys {
        val peer = decodePublic(peerPublic, (own.public as ECPublicKey).params)
        val shared = KeyAgreement.getInstance("ECDH").run {
            init(own.private)
            doPhase(peer, true)
            generateSecret()
        }
        val okm = hkdf(psk, shared, INFO + transcriptHash(desktopHello, phoneHello), 128)
        return SessionKeys(
            okm.copyOfRange(0, 32), okm.copyOfRange(32, 64),
            okm.copyOfRange(64, 96), okm.copyOfRange(96, 128),
        )
    }

    /** AES-256-GCM en un sentido; el nonce es un contador implícito (0⁴ || u64 big-endian). */
    class GcmCipher(key: ByteArray) {
        private val key = SecretKeySpec(key, "AES")
        private var counter = 0L

        private fun nextNonce(): ByteArray {
            check(counter != -1L) { "contador agotado" }
            return ByteBuffer.allocate(12).order(ByteOrder.BIG_ENDIAN).putInt(0).putLong(counter++).array()
        }

        /** Cifra con la cabecera (ya con payload_len = plaintext + 16) como AAD. */
        fun seal(header: ByteArray, plaintext: ByteArray): ByteArray =
            Cipher.getInstance("AES/GCM/NoPadding").run {
                init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(TAG_LEN * 8, nextNonce()))
                updateAAD(header)
                doFinal(plaintext)
            }

        /** Descifra; lanza AEADBadTagException si algo se manipuló, repitió o reordenó. */
        fun open(header: ByteArray, ciphertext: ByteArray): ByteArray =
            Cipher.getInstance("AES/GCM/NoPadding").run {
                init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(TAG_LEN * 8, nextNonce()))
                updateAAD(header)
                doFinal(ciphertext)
            }
    }

    fun hkdf(salt: ByteArray, ikm: ByteArray, info: ByteArray, len: Int): ByteArray {
        val prk = hmac(salt, ikm)
        val out = java.io.ByteArrayOutputStream()
        var t = ByteArray(0)
        var i = 1
        while (out.size() < len) {
            t = hmac(prk, t + info + byteArrayOf(i++.toByte()))
            out.write(t)
        }
        return out.toByteArray().copyOf(len)
    }

    fun hmac(key: ByteArray, data: ByteArray): ByteArray =
        Mac.getInstance("HmacSHA256").run { init(SecretKeySpec(key, "HmacSHA256")); doFinal(data) }

    fun sha256(b: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(b)

    fun hex(b: ByteArray): String = b.joinToString("") { "%02x".format(it.toInt() and 0xFF) }

    fun unhex(s: String): ByteArray? {
        if (s.length % 2 != 0) return null
        return ByteArray(s.length / 2) { i ->
            s.substring(2 * i, 2 * i + 2).toIntOrNull(16)?.toByte() ?: return null
        }
    }

    /** Ids hex de 4 a 64 caracteres (client_id / phone_id). */
    fun isId(s: String): Boolean = s.length in 4..64 && s.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }

    private fun be32(n: Int) = ByteBuffer.allocate(4).order(ByteOrder.BIG_ENDIAN).putInt(n).array()

    private fun fixed32(n: BigInteger): ByteArray {
        val b = n.toByteArray()
        return when {
            b.size == 32 -> b
            b.size > 32 -> b.copyOfRange(b.size - 32, b.size)
            else -> ByteArray(32 - b.size) + b
        }
    }

    // Multiplicación escalar en coordenadas afines (solo para vectores de prueba; no es de tiempo constante).
    private fun multiplyG(k: BigInteger, params: ECParameterSpec): ECPoint {
        val p = (params.curve.field as ECFieldFp).p
        val a = params.curve.a
        fun add(P: ECPoint, Q: ECPoint): ECPoint {
            if (P == ECPoint.POINT_INFINITY) return Q
            if (Q == ECPoint.POINT_INFINITY) return P
            val l = if (P == Q) {
                if (P.affineY.signum() == 0) return ECPoint.POINT_INFINITY
                P.affineX.pow(2).multiply(BigInteger.valueOf(3)).add(a)
                    .multiply(P.affineY.shiftLeft(1).modInverse(p)).mod(p)
            } else {
                if (P.affineX == Q.affineX) return ECPoint.POINT_INFINITY
                Q.affineY.subtract(P.affineY).multiply(Q.affineX.subtract(P.affineX).modInverse(p)).mod(p)
            }
            val x = l.pow(2).subtract(P.affineX).subtract(Q.affineX).mod(p)
            val y = l.multiply(P.affineX.subtract(x)).subtract(P.affineY).mod(p)
            return ECPoint(x, y)
        }
        var r = ECPoint.POINT_INFINITY
        var q = params.generator
        for (i in 0 until k.bitLength()) {
            if (k.testBit(i)) r = add(r, q)
            q = add(q, q)
        }
        return r
    }
}

/** Claves a largo plazo de los PCs emparejados (por `client_id`). */
interface PairingKeys {
    fun get(clientId: String): ByteArray?
    fun put(clientId: String, key: ByteArray)
    fun clear()
}

class MemoryPairingKeys : PairingKeys {
    private val map = HashMap<String, ByteArray>()
    @Synchronized override fun get(clientId: String) = map[clientId]
    @Synchronized override fun put(clientId: String, key: ByteArray) { map[clientId] = key }
    @Synchronized override fun clear() = map.clear()
}

/**
 * Estado de emparejamiento del móvil: id propio, código visible y claves guardadas.
 * El código cambia tras emparejar un PC y tras [Secure.MAX_CODE_ATTEMPTS] fallos seguidos,
 * así que adivinarlo por fuerza bruta en línea no es viable.
 */
class Pairing(
    val phoneId: String,
    val keys: PairingKeys,
    var onCodeChanged: ((String) -> Unit)? = null,
) {
    @Volatile var code: String = Secure.newCode()
        private set
    private var failures = 0

    @Synchronized fun onFailure() {
        if (++failures >= Secure.MAX_CODE_ATTEMPTS) rotate()
    }

    @Synchronized fun onPaired(clientId: String, key: ByteArray) {
        keys.put(clientId, key)
        rotate()
    }

    /** Olvida todos los PCs: tendrán que volver a usar el código. */
    fun forgetAll() {
        keys.clear()
        rotate()
    }

    @Synchronized fun rotate() {
        failures = 0
        code = Secure.newCode()
        onCodeChanged?.invoke(code)
    }
}
