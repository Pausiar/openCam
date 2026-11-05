package dev.opencam.app

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.Socket

class OpcmTest {
    private fun parse(bytes: ByteArray) = Opcm.read(DataInputStream(ByteArrayInputStream(bytes)))

    @Test
    fun headerRoundTrip() {
        val payload = byteArrayOf(1, 2, 3)
        val msg = parse(Opcm.encodeHeader(Opcm.VIDEO, Opcm.FLAG_KEYFRAME, 7, 123456789L, 3) + payload)
        assertEquals(Opcm.VIDEO, msg.type)
        assertEquals(Opcm.FLAG_KEYFRAME, msg.flags)
        assertEquals(7, msg.seq)
        assertEquals(123456789L, msg.timestampUs)
        assertArrayEquals(payload, msg.payload)
    }

    @Test
    fun headerIsBigEndianWithMagic() {
        val h = Opcm.encodeHeader(Opcm.STATS, 0, 1, 0, 0x01020304)
        assertEquals(Opcm.HEADER_LEN, h.size)
        assertEquals("OPCM", String(h, 0, 4))
        assertEquals(0x22, h[5].toInt())
        assertArrayEquals(byteArrayOf(1, 2, 3, 4), h.copyOfRange(20, 24))
    }

    @Test
    fun rejectsBadMagicAndOversizePayload() {
        val bad = Opcm.encodeHeader(Opcm.PING, 0, 0, 0, 0).also { it[0] = 'X'.code.toByte() }
        assertThrows(IllegalArgumentException::class.java) { parse(bad) }
        assertThrows(IllegalArgumentException::class.java) {
            parse(Opcm.encodeHeader(Opcm.VIDEO, 0, 0, 0, Opcm.MAX_PAYLOAD + 1))
        }
    }

    @Test
    fun rejectsUnsupportedVersion() {
        val h = Opcm.encodeHeader(Opcm.HELLO, 0, 0, 0, 0).also { it[4] = 2 }
        assertEquals(2, assertThrows(Opcm.UnsupportedVersion::class.java) { parse(h) }.version)
    }

    @Test
    fun fpsRangePrefersNarrowestContaining() {
        val r = listOf(15 to 30, 30 to 30, 24 to 60)
        assertEquals(30 to 30, Caps.pickFpsRange(r, 30))
        assertEquals(24 to 60, Caps.pickFpsRange(r, 60))
        assertEquals(15 to 30, Caps.pickFpsRange(listOf(15 to 30), 60)) // ninguno contiene 60: el más cercano
        assertEquals(null, Caps.pickFpsRange(emptyList(), 30))
        assertEquals(listOf(30, 60), Caps.fpsList(r + (7 to 10)))
    }

    @Test
    fun lensNames() {
        assertEquals("ultrawide", Caps.lensName(13f))
        assertEquals("wide", Caps.lensName(26f))
        assertEquals("tele", Caps.lensName(70f))
    }

    private fun serverFor(onKey: () -> Unit = {}) =
        OpcmServer(0, { "{\"platform\":\"android\"}" }, {}, {}, onKey, {}).also { it.start() }

    private fun send(out: DataOutputStream, type: Int, version: Int = 1) {
        out.write(Opcm.encodeHeader(type, 0, 0, 42, 0).also { it[4] = version.toByte() })
        out.flush()
    }

    @Test
    fun helloGetsHelloCapsAndPongEchoesTimestamp() {
        var keyRequests = 0
        val server = serverFor { keyRequests++ }
        try {
            Socket("127.0.0.1", server.localPort).use { s ->
                s.soTimeout = 5000
                val out = DataOutputStream(s.getOutputStream())
                val inp = DataInputStream(s.getInputStream())
                send(out, Opcm.HELLO)
                assertEquals(Opcm.HELLO, Opcm.read(inp).type)
                assertEquals(Opcm.CAPS, Opcm.read(inp).type)
                send(out, Opcm.PING)
                val pong = Opcm.read(inp)
                assertEquals(Opcm.PONG, pong.type)
                assertEquals(42L, pong.timestampUs)
                assertTrue(keyRequests >= 1)
            }
        } finally {
            server.stop()
        }
    }

    @Test
    fun unsupportedVersionGetsErrorThenClose() {
        val server = serverFor()
        try {
            Socket("127.0.0.1", server.localPort).use { s ->
                s.soTimeout = 5000
                send(DataOutputStream(s.getOutputStream()), Opcm.HELLO, version = 9)
                val inp = DataInputStream(s.getInputStream())
                val err = Opcm.read(inp)
                assertEquals(Opcm.ERROR, err.type)
                assertTrue(String(err.payload).contains("unsupported_version"))
                assertEquals(-1, inp.read()) // el servidor cierra
            }
        } finally {
            server.stop()
        }
    }

    @Test
    fun lateClientDropsPFramesAndRequestsKeyframe() {
        var keyRequests = 0
        val server = serverFor { keyRequests++ }
        try {
            Socket("127.0.0.1", server.localPort).use { s ->
                s.soTimeout = 5000
                val inp = DataInputStream(s.getInputStream())
                DataOutputStream(s.getOutputStream()).also { send(it, Opcm.HELLO) }
                Opcm.read(inp); Opcm.read(inp) // HELLO, CAPS
                // Cliente nuevo: hasta el primer keyframe se descartan los P-frames.
                assertEquals(false, server.sendVideo(ByteArray(1), 1, keyframe = false))
                assertEquals(true, server.sendVideo(ByteArray(1), 2, keyframe = true))
                val v = Opcm.read(inp)
                assertEquals(Opcm.VIDEO, v.type)
                assertEquals(Opcm.FLAG_KEYFRAME, v.flags)
            }
        } finally {
            server.stop()
        }
    }
}
