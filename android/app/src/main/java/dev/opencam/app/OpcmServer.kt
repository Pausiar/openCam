package dev.opencam.app

import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.ByteBuffer
import java.nio.ByteOrder

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
    const val VIDEO = 0x10
    const val PING = 0x20
    const val PONG = 0x21
    const val STATS = 0x22
    const val KEYFRAME_REQUEST = 0x30
    const val ERROR = 0x7F

    val MAGIC = byteArrayOf('O'.code.toByte(), 'P'.code.toByte(), 'C'.code.toByte(), 'M'.code.toByte())

    class UnsupportedVersion(val version: Int) : Exception("versión no soportada: $version")

    class Message(val type: Int, val flags: Int, val seq: Int, val timestampUs: Long, val payload: ByteArray)

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

    fun read(input: DataInputStream): Message {
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
        require(len in 0..MAX_PAYLOAD) { "payload fuera de rango: $len" }
        val payload = ByteArray(len)
        input.readFully(payload)
        return Message(type, flags, seq, ts, payload)
    }
}

/**
 * Servidor TCP en el móvil (el escritorio conecta, vía `adb forward` o Wi-Fi).
 * Solo admite un cliente a la vez; una nueva conexión sustituye a la anterior.
 * Todo el envío pasa por una cola y un hilo escritor: quien llama nunca bloquea en el socket.
 */
class OpcmServer(
    private val port: Int = Opcm.DEFAULT_PORT,
    private val capsJson: () -> String,
    private val onConfig: (String) -> Unit,
    private val onStats: (String) -> Unit,
    private val onKeyframeRequest: () -> Unit,
    private val onClientChanged: (connected: Boolean) -> Unit,
) {
    private class Out(val type: Int, val flags: Int, val ts: Long, val payload: ByteArray, val closeAfter: Boolean = false)

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
            disconnect()
            runCatching { sock.tcpNoDelay = true }
            synchronized(lock) {
                client = sock
                queue.clear()
                waitingKey = true // el cliente nuevo debe empezar por un keyframe
            }
            onClientChanged(true)
            onKeyframeRequest()
            Thread({ writeLoop(sock) }, "opcm-write").start()
            Thread({ readLoop(sock) }, "opcm-read").start()
        }
    }

    private fun writeLoop(sock: Socket) {
        try {
            val o = BufferedOutputStream(sock.getOutputStream())
            var seq = 0
            while (true) {
                val m = synchronized(lock) {
                    while (client === sock && queue.isEmpty()) lock.wait()
                    if (client !== sock) return
                    queue.removeFirst()
                }
                o.write(Opcm.encodeHeader(m.type, m.flags, seq++, m.ts, m.payload.size))
                o.write(m.payload)
                o.flush()
                if (m.closeAfter) break
            }
        } catch (e: Exception) {
            // socket cerrado o error de escritura
        } finally {
            disconnect(sock)
        }
    }

    private fun readLoop(sock: Socket) {
        try {
            val input = DataInputStream(BufferedInputStream(sock.getInputStream()))
            while (running && !sock.isClosed) {
                val msg = Opcm.read(input)
                when (msg.type) {
                    Opcm.HELLO -> {
                        send(Opcm.HELLO, 0, nowUs(), "{\"proto\":1,\"platform\":\"android\"}".toByteArray())
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
        "{\"code\":\"$code\",\"message\":\"${message.replace("\\", "\\\\").replace("\"", "\\\"")}\"}".toByteArray()

    companion object {
        // shortcut: umbral fijo (~100 ms a 30 fps), hacerlo dependiente del bitrate si hace falta.
        private const val MAX_PENDING_VIDEO = 3
    }
}
