package dev.opencam.app

import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
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
    const val KEYFRAME_REQUEST = 0x30
    const val ERROR = 0x7F

    val MAGIC = byteArrayOf('O'.code.toByte(), 'P'.code.toByte(), 'C'.code.toByte(), 'M'.code.toByte())

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
        require(version == VERSION) { "versión no soportada: $version" }
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
 */
class OpcmServer(
    private val port: Int = Opcm.DEFAULT_PORT,
    private val capsJson: () -> String,
    private val onConfig: (String) -> Unit,
    private val onKeyframeRequest: () -> Unit,
    private val onClientChanged: (connected: Boolean) -> Unit,
) {
    private val lock = Any()
    private var server: ServerSocket? = null
    private var client: Socket? = null
    private var out: BufferedOutputStream? = null
    private var seq = 0
    @Volatile private var running = false

    fun start() {
        running = true
        val s = ServerSocket(port).also { it.reuseAddress = true }
        server = s
        Thread({ acceptLoop(s) }, "opcm-accept").start()
    }

    fun stop() {
        running = false
        runCatching { server?.close() }
        disconnect()
    }

    /** Envía un mensaje; devuelve false si no hay cliente o falla la escritura. */
    fun send(type: Int, flags: Int, timestampUs: Long, payload: ByteArray = ByteArray(0)): Boolean = synchronized(lock) {
        val o = out ?: return false
        return try {
            o.write(Opcm.encodeHeader(type, flags, seq++, timestampUs, payload.size))
            o.write(payload)
            o.flush()
            true
        } catch (e: Exception) {
            disconnect()
            false
        }
    }

    fun sendVideo(data: ByteArray, timestampUs: Long, keyframe: Boolean): Boolean =
        send(Opcm.VIDEO, if (keyframe) Opcm.FLAG_KEYFRAME else 0, timestampUs, data)

    private fun acceptLoop(s: ServerSocket) {
        while (running) {
            val sock = try { s.accept() } catch (e: Exception) { break }
            disconnect()
            synchronized(lock) {
                client = sock
                out = BufferedOutputStream(sock.getOutputStream())
            }
            onClientChanged(true)
            Thread({ readLoop(sock) }, "opcm-read").start()
        }
    }

    private fun readLoop(sock: Socket) {
        try {
            val input = DataInputStream(BufferedInputStream(sock.getInputStream()))
            while (running && !sock.isClosed) {
                val msg = Opcm.read(input)
                when (msg.type) {
                    Opcm.HELLO -> {
                        send(Opcm.HELLO, 0, System.nanoTime() / 1000, "{\"proto\":1,\"platform\":\"android\"}".toByteArray())
                        send(Opcm.CAPS, 0, System.nanoTime() / 1000, capsJson().toByteArray())
                    }
                    Opcm.CONFIG -> onConfig(String(msg.payload))
                    Opcm.PING -> send(Opcm.PONG, 0, msg.timestampUs)
                    Opcm.KEYFRAME_REQUEST -> onKeyframeRequest()
                    else -> Unit // tipos desconocidos: se ignoran
                }
            }
        } catch (e: Exception) {
            // Conexión cerrada o protocolo inválido: se desconecta y se espera un nuevo cliente.
        } finally {
            synchronized(lock) {
                if (client === sock) disconnect()
            }
        }
    }

    private fun disconnect() {
        val wasConnected: Boolean
        synchronized(lock) {
            wasConnected = client != null
            runCatching { client?.close() }
            client = null
            out = null
        }
        if (wasConnected) onClientChanged(false)
    }
}
