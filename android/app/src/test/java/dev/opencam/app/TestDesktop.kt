package dev.opencam.app

import java.io.Closeable
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.Socket
import org.json.JSONObject

/**
 * Escritorio mínimo para los tests: hace el mismo handshake que `desktop/src/client.rs`
 * (HELLO → HELLO → AUTH → AUTH) y luego cifra/descifra con las claves de sesión.
 * Lanza [Rejected] si el móvil responde ERROR.
 */
class TestDesktop(
    port: Int,
    code: String? = null,
    savedKey: ByteArray? = null,
    val clientId: String = "c0ffee00",
) : Closeable {
    class Rejected(val code: String) : Exception(code)

    private val sock = Socket("127.0.0.1", port).apply { soTimeout = 5000 }
    private val inp = DataInputStream(sock.getInputStream())
    private val out = DataOutputStream(sock.getOutputStream())
    private val tx: Secure.GcmCipher
    private val rx: Secure.GcmCipher
    private var seq = 0
    val authMode: String
    val pairingKey: ByteArray
    var lastCiphertext = ByteArray(0)
        private set

    init {
        val kp = Secure.generateKeyPair()
        val hello = JSONObject()
            .put("proto", 1).put("platform", "desktop").put("secure", 1)
            .put("client_id", clientId).put("pub", Secure.hex(Secure.publicBytes(kp)))
            .put("auth", if (code != null) "code" else "key")
            .toString().toByteArray()
        writePlain(Opcm.HELLO, hello)
        val reply = readPlain()
        val r = JSONObject(String(reply.payload))
        authMode = r.getString("auth")
        val psk = if (authMode == "code") Secure.codePsk(requireNotNull(code)) else requireNotNull(savedKey)
        val keys = Secure.derive(kp, Secure.unhex(r.getString("pub"))!!, psk, hello, reply.payload)
        writePlain(Opcm.AUTH, keys.authTag(Secure.Role.DESKTOP))
        val auth = readPlain()
        keys.verifyAuth(Secure.Role.PHONE, auth.payload)
        pairingKey = keys.pairingKey
        val (t, x) = keys.ciphers(Secure.Role.DESKTOP)
        tx = t
        rx = x
    }

    private fun readPlain(): Opcm.Message {
        val m = Opcm.read(inp)
        if (m.type == Opcm.ERROR) {
            sock.close()
            throw Rejected(JSONObject(String(m.payload)).optString("code"))
        }
        return m
    }

    private fun writePlain(type: Int, payload: ByteArray) {
        out.write(Opcm.encodeHeader(type, 0, seq++, 0, payload.size))
        out.write(payload)
        out.flush()
    }

    fun send(type: Int, payload: ByteArray = ByteArray(0), ts: Long = 0) {
        val h = Opcm.encodeHeader(type, 0, seq++, ts, payload.size + Secure.TAG_LEN)
        out.write(h)
        out.write(tx.seal(h, payload))
        out.flush()
    }

    /** Envía un mensaje con un bit del texto cifrado cambiado. */
    fun sendTampered(type: Int) {
        val h = Opcm.encodeHeader(type, 0, seq++, 0, Secure.TAG_LEN)
        val ct = tx.seal(h, ByteArray(0)).also { it[0] = (it[0].toInt() xor 1).toByte() }
        out.write(h)
        out.write(ct)
        out.flush()
    }

    fun recv(): Opcm.Message {
        val m = Opcm.read(inp)
        lastCiphertext = m.payload
        return Opcm.Message(m.type, m.flags, m.seq, m.timestampUs, rx.open(m.header, m.payload))
    }

    fun rawRead(): Int = inp.read()

    override fun close() = sock.close()
}
