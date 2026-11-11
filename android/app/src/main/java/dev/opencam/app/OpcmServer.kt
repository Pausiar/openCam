package dev.opencam.app

import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.OutputStream
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicInteger
import org.json.JSONObject

/** Implementa el framing OPCM v1 de docs/PROTOCOL.md (cabecera de 24 bytes, big-endian). */
object Opcm {
    const val VERSION = 1
    const val HEADER_LEN = 24
    const val MAX_PAYLOAD = 16 * 1024 * 1024
    const val DEFAULT_PORT = 4747
    const val FLAG_KEYFRAME = 0x0001

    const val HELLO = 0x01
    const val CAPS = 0x02
    const val CONFIG = 0x03
    const val AUTH = 0x04
    const val VIDEO = 0x10
    const val PING = 0x20
    const val PONG = 0x21
    const val STATS = 0x22
    const val KEYFRAME_REQUEST = 0x30
    const val ERROR = 0x7F

    val MAGIC = byteArrayOf('O'.code.toByte(), 'P'.code.toByte(), 'C'.code.toByte(), 'M'.code.toByte())

    class UnsupportedVersion(val version: Int) : Exception("versión no soportada: $version")

    /** `header` son los 24 bytes tal y como llegaron (AAD para descifrar). */
    class Message(val type: Int, val flags: Int, val seq: Int, val timestampUs: Long, val payload: ByteArray, val header: ByteArray = ByteArray(0))

    fun encodeHeader(type: Int, flags: Int, seq: Int, timestampUs: Long, payloadLen: Int): ByteArray =
        ByteBuffer.allocate(HEADER_LEN).order(ByteOrder.BIG_ENDIAN).apply {
            put(MAGIC)
            put(VERSION.toByte())
            put(type.toByte())
            putShort(flags.toShort())
            putInt(seq)
            putLong(timestampUs)
            putInt(payloadLen)
        }.array()

    /** Lee un mensaje; `maxPayload` se valida antes de reservar memoria. */
    fun read(input: DataInputStream, maxPayload: Int = MAX_PAYLOAD): Message {
        val h = ByteArray(HEADER_LEN)
        input.readFully(h)
        val b = ByteBuffer.wrap(h).order(ByteOrder.BIG_ENDIAN)
        val magic = ByteArray(4).also { b.get(it) }
        require(magic.contentEquals(MAGIC)) { "magic inválido" }
        val version = b.get().toInt() and 0xFF
        if (version != VERSION) throw UnsupportedVersion(version)
        val type = b.get().toInt() and 0xFF
        val flags = b.short.toInt() and 0xFFFF
        val seq = b.int
        val ts = b.long
        val len = b.int
        require(len in 0..minOf(maxPayload, MAX_PAYLOAD)) { "payload fuera de rango: $len" }
        val payload = ByteArray(len)
        input.readFully(payload)
        return Message(type, flags, seq, ts, payload, h)
    }
}

/**
 * Servidor TCP en el móvil (el escritorio conecta, vía `adb forward` o Wi-Fi).
 * Cada conexión debe emparejarse (código o clave guardada, ver [Pairing]) antes de recibir
 * nada; solo entonces sustituye al cliente anterior, así que un extraño en la red no puede
 * ver la cámara ni echar al PC conectado. Tras AUTH todo va cifrado con AES-GCM.
 * Todo el envío pasa por una cola y un hilo escritor: quien llama nunca bloquea en el socket.
 */
class OpcmServer(
    private val port: Int = Opcm.DEFAULT_PORT,
    private val capsJson: () -> String,
    private val onConfig: (String) -> Unit,
    private val onStats: (String) -> Unit,
    private val onKeyframeRequest: () -> Unit,
    private val onClientChanged: (connected: Boolean) -> Unit,
    private val pairing: Pairing,
    private val platform: String = "android",
) {
    private class Out(val type: Int, val flags: Int, val ts: Long, val payload: ByteArray, val closeAfter: Boolean = false)

    private val pendingHandshakes = AtomicInteger(0)

    private val lock = Object()
    private var server: ServerSocket? = null
    private var client: Socket? = null
    private val queue = ArrayDeque<Out>() // protegida por lock
    private var waitingKey = false // tras un descarte, se tiran P-frames hasta el siguiente keyframe
    @Volatile private var running = false

    /** Puerto realmente enlazado (útil con port = 0). */
    val localPort: Int get() = server?.localPort ?: -1

    fun start() {
        running = true
        // reuseAddress debe fijarse antes del bind.
        val s = ServerSocket().apply {
            reuseAddress = true
            bind(InetSocketAddress(port))
        }
        server = s
        Thread({ acceptLoop(s) }, "opcm-accept").start()
    }

    fun stop() {
        running = false
        runCatching { server?.close() }
        disconnect()
    }

    /** Encola un mensaje de control (nunca se descarta); false si no hay cliente. */
    fun send(type: Int, flags: Int, timestampUs: Long, payload: ByteArray = ByteArray(0)): Boolean =
        enqueue(Out(type, flags, timestampUs, payload))

    fun sendError(code: String, message: String): Boolean =
        enqueue(Out(Opcm.ERROR, 0, nowUs(), errorJson(code, message)))

    /**
     * Encola un frame. Si la cola de vídeo va retrasada se descarta y se pide keyframe;
     * un keyframe nuevo sustituye a los frames pendientes.
     */
    fun sendVideo(data: ByteArray, timestampUs: Long, keyframe: Boolean): Boolean {
        var needKey = false
        val ok = synchronized(lock) {
            if (client == null) return false
            if (keyframe) {
                queue.removeAll { it.type == Opcm.VIDEO }
                waitingKey = false
            } else if (waitingKey) {
                return false
            } else if (queue.count { it.type == Opcm.VIDEO } >= MAX_PENDING_VIDEO) {
                queue.removeAll { it.type == Opcm.VIDEO }
                waitingKey = true
                needKey = true
            }
            if (!waitingKey) {
                queue.addLast(Out(Opcm.VIDEO, if (keyframe) Opcm.FLAG_KEYFRAME else 0, timestampUs, data))
                lock.notifyAll()
                true
            } else false
        }
        if (needKey) onKeyframeRequest()
        return ok
    }

    private fun enqueue(m: Out): Boolean = synchronized(lock) {
        if (client == null) return false
        queue.addLast(m)
        lock.notifyAll()
        true
    }

    private fun acceptLoop(s: ServerSocket) {
        while (running) {
            val sock = try { s.accept() } catch (e: Exception) { break }
            // Pocos handshakes a la vez: nadie agota hilos abriendo conexiones mudas.
            if (pendingHandshakes.incrementAndGet() > MAX_PENDING_HANDSHAKES) {
                pendingHandshakes.decrementAndGet()
                runCatching { sock.close() }
                continue
            }
            Thread({
                try { handshake(sock) } finally { pendingHandshakes.decrementAndGet() }
            }, "opcm-handshake").start()
        }
    }

    /** HELLO → HELLO → AUTH → AUTH, en claro y con tiempo límite; si sale bien, la conexión pasa a ser el cliente. */
    private fun handshake(sock: Socket) {
        try {
            sock.soTimeout = HANDSHAKE_TIMEOUT_MS
            runCatching { sock.tcpNoDelay = true }
            val input = DataInputStream(BufferedInputStream(sock.getInputStream()))
            val out = BufferedOutputStream(sock.getOutputStream())
            var seq = 0
            fun write(type: Int, payload: ByteArray) {
                out.write(Opcm.encodeHeader(type, 0, seq++, nowUs(), payload.size))
                out.write(payload)
                out.flush()
            }
            fun refuse(code: String, message: String) {
                runCatching { write(Opcm.ERROR, errorJson(code, message)) }
                sock.close()
            }

            val hello = try {
                Opcm.read(input, Secure.MAX_UNAUTH_PAYLOAD)
            } catch (e: Opcm.UnsupportedVersion) {
                return refuse("unsupported_version", "versión ${e.version} no soportada")
            }
            if (hello.type != Opcm.HELLO) return sock.close()
            val req = runCatching { JSONObject(String(hello.payload)) }.getOrNull() ?: JSONObject()
            if (req.optInt("secure", 0) != 1) {
                return refuse("pairing_required", "Este móvil exige emparejamiento cifrado: actualiza OpenCam en el PC")
            }
            val clientId = req.optString("client_id")
            val peer = Secure.unhex(req.optString("pub"))
            if (!Secure.isId(clientId) || peer == null) return refuse("bad_hello", "HELLO incompleto")

            val savedKey = if (req.optString("auth") == "key") pairing.keys.get(clientId) else null
            if (savedKey == null) {
                // Tras varios códigos fallidos seguidos se deja de aceptar código durante un rato.
                val wait = pairing.lockedForMs()
                if (wait > 0) {
                    return refuse("too_many_attempts", "Demasiados códigos incorrectos: espera ${(wait + 999) / 1000} s y usa el código nuevo que muestra el móvil")
                }
            }
            val code = pairing.code
            val kp = Secure.generateKeyPair()
            val reply = JSONObject()
                .put("proto", 1).put("platform", platform).put("secure", 1)
                .put("phone_id", pairing.phoneId)
                .put("pub", Secure.hex(Secure.publicBytes(kp)))
                .put("auth", if (savedKey != null) "key" else "code")
                .toString().toByteArray()
            write(Opcm.HELLO, reply)

            val keys = try {
                Secure.derive(kp, peer, savedKey ?: Secure.codePsk(code), hello.payload, reply)
            } catch (e: IllegalArgumentException) {
                return refuse("bad_hello", "clave pública inválida")
            }
            val auth = Opcm.read(input, Secure.MAX_UNAUTH_PAYLOAD)
            if (auth.type != Opcm.AUTH) return sock.close()
            try {
                keys.verifyAuth(Secure.Role.DESKTOP, auth.payload)
            } catch (e: Secure.AuthFailed) {
                if (savedKey == null) {
                    pairing.onFailure()
                    return refuse("bad_code", "código de emparejamiento incorrecto")
                }
                return refuse("bad_key", "este PC ya no está emparejado; usa el código")
            }
            // Se guarda la clave antes de confirmar: cuando el PC recibe AUTH ya puede reconectar con ella.
            if (savedKey == null) pairing.onPaired(clientId, keys.pairingKey)
            write(Opcm.AUTH, keys.authTag(Secure.Role.PHONE))
            // El PC manda un PING por segundo: si calla este tiempo, la conexión murió sin cerrarse
            // (Wi-Fi caída, portátil suspendido) y hay que soltarla para no dar por conectado a nadie.
            sock.soTimeout = CLIENT_TIMEOUT_MS
            val (tx, rx) = keys.ciphers(Secure.Role.PHONE)
            promote(sock, input, out, tx, rx, seq)
        } catch (e: Exception) {
            // Timeout, conexión cerrada o datos inválidos: se descarta sin tocar al cliente actual.
            runCatching { sock.close() }
        }
    }

    private fun promote(sock: Socket, input: DataInputStream, out: OutputStream, tx: Secure.GcmCipher, rx: Secure.GcmCipher, seq: Int) {
        if (!running) { runCatching { sock.close() }; return }
        disconnect()
        synchronized(lock) {
            client = sock
            queue.clear()
            waitingKey = true // el cliente nuevo debe empezar por un keyframe
        }
        onClientChanged(true)
        Thread({ writeLoop(sock, out, tx, seq) }, "opcm-write").start()
        Thread({ readLoop(sock, input, rx) }, "opcm-read").start()
        send(Opcm.CAPS, 0, nowUs(), capsJson().toByteArray())
        onKeyframeRequest()
    }

    private fun writeLoop(sock: Socket, o: OutputStream, tx: Secure.GcmCipher, firstSeq: Int) {
        try {
            var seq = firstSeq
            while (true) {
                val m = synchronized(lock) {
                    while (client === sock && queue.isEmpty()) lock.wait()
                    if (client !== sock) return
                    queue.removeFirst()
                }
                val header = Opcm.encodeHeader(m.type, m.flags, seq++, m.ts, m.payload.size + Secure.TAG_LEN)
                o.write(header)
                o.write(tx.seal(header, m.payload))
                o.flush()
                if (m.closeAfter) break
            }
        } catch (e: Exception) {
            // socket cerrado o error de escritura
        } finally {
            disconnect(sock)
        }
    }

    private fun readLoop(sock: Socket, input: DataInputStream, rx: Secure.GcmCipher) {
        try {
            while (running && !sock.isClosed) {
                val raw = Opcm.read(input)
                // Un mensaje manipulado, repetido o sin cifrar lanza excepción y corta la conexión.
                val payload = rx.open(raw.header, raw.payload)
                val msg = Opcm.Message(raw.type, raw.flags, raw.seq, raw.timestampUs, payload)
                when (msg.type) {
                    Opcm.HELLO -> {
                        send(Opcm.CAPS, 0, nowUs(), capsJson().toByteArray())
                        onKeyframeRequest()
                    }
                    Opcm.CONFIG -> onConfig(String(msg.payload))
                    Opcm.STATS -> onStats(String(msg.payload))
                    Opcm.PING -> send(Opcm.PONG, 0, msg.timestampUs)
                    Opcm.KEYFRAME_REQUEST -> onKeyframeRequest()
                    else -> Unit // tipos desconocidos: se ignoran
                }
            }
        } catch (e: Opcm.UnsupportedVersion) {
            // ERROR y cierre una vez vaciado lo ya encolado.
            enqueue(Out(Opcm.ERROR, 0, nowUs(), errorJson("unsupported_version", "versión ${e.version} no soportada"), closeAfter = true))
            return
        } catch (e: Exception) {
            // Conexión cerrada o protocolo inválido: se desconecta y se espera un nuevo cliente.
        }
        disconnect(sock)
    }

    /** Cierra el cliente actual (o solo `only`, si sigue siendo el actual). */
    private fun disconnect(only: Socket? = null) {
        val wasConnected: Boolean
        synchronized(lock) {
            if (only != null && client !== only) return
            wasConnected = client != null
            runCatching { client?.close() }
            client = null
            queue.clear()
            lock.notifyAll()
        }
        if (wasConnected) onClientChanged(false)
    }

    private fun nowUs() = System.nanoTime() / 1000

    private fun errorJson(code: String, message: String) =
        JSONObject().put("code", code).put("message", message).toString().toByteArray()

    companion object {
        // shortcut: umbral fijo (~100 ms a 30 fps), hacerlo dependiente del bitrate si hace falta.
        private const val MAX_PENDING_VIDEO = 3
        private const val MAX_PENDING_HANDSHAKES = 4
        private const val HANDSHAKE_TIMEOUT_MS = 10_000
        private const val CLIENT_TIMEOUT_MS = 10_000
    }
}
