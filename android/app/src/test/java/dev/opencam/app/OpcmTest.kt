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

    @Test
    fun readRespectsCustomLimitAndKeepsRawHeader() {
        val h = Opcm.encodeHeader(Opcm.HELLO, 0, 0, 0, Secure.MAX_UNAUTH_PAYLOAD + 1)
        assertThrows(IllegalArgumentException::class.java) {
            Opcm.read(DataInputStream(ByteArrayInputStream(h)), Secure.MAX_UNAUTH_PAYLOAD)
        }
        val ok = Opcm.encodeHeader(Opcm.PING, 0, 5, 9, 0)
        assertArrayEquals(ok, parse(ok).header)
    }

    @Test
    fun streamConfigClampsAndValidates() {
        val fb = StreamConfig()
        val c = StreamConfig.parse("""{"width":1281,"height":721,"fps":60,"bitrate_kbps":4000,"codec":"h265","lens":"tele"}""", fb)
        assertEquals(StreamConfig(1280, 720, 60, 4000, "h265", "tele"), c)
        val bad = StreamConfig.parse("""{"width":-5,"height":99999,"fps":0,"bitrate_kbps":999999999,"codec":"evil","lens":"../x"}""", fb)
        assertEquals(fb.width, bad.width)
        assertEquals(StreamConfig.MAX_DIMENSION, bad.height)
        assertEquals(fb.fps, bad.fps)
        assertEquals(StreamConfig.MAX_BITRATE_KBPS, bad.bitrateKbps)
        assertEquals(fb.codec, bad.codec)
        assertEquals(fb.lens, bad.lens)
        assertEquals(StreamConfig.MIN_DIMENSION, StreamConfig.parse("""{"width":1}""", fb).width)
        assertEquals(StreamConfig.MAX_FPS, StreamConfig.parse("""{"fps":1000}""", fb).fps)
        assertEquals(fb, StreamConfig.parse("{}", fb))
    }

    // --- Servidor: emparejamiento, cifrado y robustez, con un escritorio de prueba ---

    private fun serverFor(pairing: Pairing = Pairing("a1b2c3d4", MemoryPairingKeys()), onKey: () -> Unit = {}) =
        OpcmServer(0, { "{\"platform\":\"android\"}" }, {}, {}, onKey, {}, pairing).also { it.start() }

    private fun send(out: DataOutputStream, type: Int, version: Int = 1) {
        out.write(Opcm.encodeHeader(type, 0, 0, 42, 0).also { it[4] = version.toByte() })
        out.flush()
    }

    @Test
    fun pairsWithCodeThenEncryptsEverything() {
        var keyRequests = 0
        val pairing = Pairing("a1b2c3d4", MemoryPairingKeys())
        var rotations = 0
        pairing.onCodeChanged = { rotations++ }
        val server = serverFor(pairing) { keyRequests++ }
        try {
            val code = pairing.code
            val d = TestDesktop(server.localPort, code = code)
            d.use {
                assertEquals("code", d.authMode)
                val caps = d.recv()
                assertEquals(Opcm.CAPS, caps.type)
                assertTrue(String(caps.payload).contains("android"))
                assertTrue(d.lastCiphertext.size == caps.payload.size + Secure.TAG_LEN)
                d.send(Opcm.PING, ts = 42)
                val pong = d.recv()
                assertEquals(Opcm.PONG, pong.type)
                assertEquals(42L, pong.timestampUs)
                assertTrue(keyRequests >= 1)
            }
            // El PC queda recordado con la misma clave en ambos lados y el código cambia.
            assertArrayEquals(d.pairingKey, pairing.keys.get(d.clientId))
            assertEquals(1, rotations)
            TestDesktop(server.localPort, savedKey = d.pairingKey, clientId = d.clientId).use { d2 ->
                assertEquals("key", d2.authMode)
                assertEquals(Opcm.CAPS, d2.recv().type)
            }
        } finally {
            server.stop()
        }
    }

    @Test
    fun wrongCodeIsRejectedAndCodeRotatesAfterFiveFailures() {
        val pairing = Pairing("a1b2c3d4", MemoryPairingKeys())
        var rotations = 0
        pairing.onCodeChanged = { rotations++ }
        val server = serverFor(pairing)
        try {
            repeat(Secure.MAX_CODE_ATTEMPTS) {
                val wrong = ((pairing.code.toInt() + 1) % 1_000_000).toString().padStart(6, '0')
                val err = assertThrows(TestDesktop.Rejected::class.java) { TestDesktop(server.localPort, code = wrong) }
                assertEquals("bad_code", err.code)
            }
            assertEquals(1, rotations)
            assertEquals(null, pairing.keys.get("c0ffee00"))
        } finally {
            server.stop()
        }
    }

    @Test
    fun legacyDesktopIsAskedToUpdate() {
        val server = serverFor()
        try {
            Socket("127.0.0.1", server.localPort).use { s ->
                s.soTimeout = 5000
                send(DataOutputStream(s.getOutputStream()), Opcm.HELLO)
                val inp = DataInputStream(s.getInputStream())
                val err = Opcm.read(inp)
                assertEquals(Opcm.ERROR, err.type)
                assertTrue(String(err.payload).contains("pairing_required"))
                assertEquals(-1, inp.read())
            }
        } finally {
            server.stop()
        }
    }

    @Test
    fun strangerCannotKickPairedClient() {
        val pairing = Pairing("a1b2c3d4", MemoryPairingKeys())
        val server = serverFor(pairing)
        try {
            TestDesktop(server.localPort, code = pairing.code).use { d ->
                assertEquals(Opcm.CAPS, d.recv().type)
                // Un extraño se conecta y no sabe el código: el cliente emparejado sigue.
                assertThrows(TestDesktop.Rejected::class.java) { TestDesktop(server.localPort, code = "000000".takeIf { it != pairing.code } ?: "111111") }
                Socket("127.0.0.1", server.localPort).use { s -> s.getOutputStream().write(ByteArray(10)) }
                d.send(Opcm.PING, ts = 7)
                assertEquals(7L, d.recv().timestampUs)
            }
        } finally {
            server.stop()
        }
    }

    @Test
    fun oversizedPayloadBeforeAuthIsDropped() {
        val server = serverFor()
        try {
            Socket("127.0.0.1", server.localPort).use { s ->
                s.soTimeout = 5000
                s.getOutputStream().write(Opcm.encodeHeader(Opcm.HELLO, 0, 0, 0, Secure.MAX_UNAUTH_PAYLOAD + 1))
                assertEquals(-1, s.getInputStream().read()) // cierra sin responder
            }
        } finally {
            server.stop()
        }
    }

    @Test
    fun tamperedMessageClosesConnection() {
        val pairing = Pairing("a1b2c3d4", MemoryPairingKeys())
        val server = serverFor(pairing)
        try {
            TestDesktop(server.localPort, code = pairing.code).use { d ->
                assertEquals(Opcm.CAPS, d.recv().type)
                d.sendTampered(Opcm.PING)
                assertEquals(-1, d.rawRead())
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
        val pairing = Pairing("a1b2c3d4", MemoryPairingKeys())
        val server = serverFor(pairing) { keyRequests++ }
        try {
            TestDesktop(server.localPort, code = pairing.code).use { d ->
                assertEquals(Opcm.CAPS, d.recv().type)
                // Cliente nuevo: hasta el primer keyframe se descartan los P-frames.
                assertEquals(false, server.sendVideo(ByteArray(1), 1, keyframe = false))
                assertEquals(true, server.sendVideo(ByteArray(1), 2, keyframe = true))
                val v = d.recv()
                assertEquals(Opcm.VIDEO, v.type)
                assertEquals(Opcm.FLAG_KEYFRAME, v.flags)
                assertTrue(keyRequests >= 1)
            }
        } finally {
            server.stop()
        }
    }
}
