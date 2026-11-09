package dev.opencam.app

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import java.security.interfaces.ECPublicKey
import javax.crypto.AEADBadTagException

class SecureTest {
    private fun hex(b: ByteArray) = Secure.hex(b)

    /**
     * Mismo vector que `core/src/secure.rs` (tests::vector): garantiza que Android y el
     * escritorio derivan las mismas claves y cifran igual.
     */
    @Test
    fun matchesRustVector() {
        val d = Secure.keyPairFromPrivate(ByteArray(32) { 0x11 })
        val p = Secure.keyPairFromPrivate(ByteArray(32) { 0x22 })
        val psk = Secure.codePsk("123456")
        val k = Secure.derive(d, Secure.publicBytes(p), psk, """{"d":1}""".toByteArray(), """{"p":1}""".toByteArray())
        val header = Opcm.encodeHeader(Opcm.CONFIG, 0, 3, 99, 4 + Secure.TAG_LEN)
        val ct = k.ciphers(Secure.Role.DESKTOP).first.seal(header, "hola".toByteArray())
        assertEquals(
            listOf(
                "040217e617f0b6443928278f96999e69a23a4f2c152bdf6d6cdf66e5b80282d4ed194a7debcb97712d2dda3ca85aa8765a56f45fc758599652f2897c65306e5794",
                "ea64913738e9cba75bb0b11f5762959a7a65e75f4dadc994cbe65c98a65e35ae",
                "5c91459ea4bd095b6654962bb8c378c74f913b7dc625787be52d40d58fd697d6",
                "26e992cdf4b6ce8226a89703bfcc829c31df2aa5073a04c81da1442c330489c3",
                "9083aaa25d1afa6d4c90608dac80551c531debb56de6a4af6bb2bcbe06ef6fe8",
                "4f50434d0103000000000003000000000000006300000014",
                "98d8b0415dcaf17c08ad6e0d3f0ff53487dc0976",
            ),
            listOf(
                hex(Secure.publicBytes(d)), hex(psk),
                hex(k.authTag(Secure.Role.DESKTOP)), hex(k.authTag(Secure.Role.PHONE)),
                hex(k.pairingKey), hex(header), hex(ct),
            ),
        )
        // El móvil, con su clave privada, llega a lo mismo.
        val kp = Secure.derive(p, Secure.publicBytes(d), psk, """{"d":1}""".toByteArray(), """{"p":1}""".toByteArray())
        kp.verifyAuth(Secure.Role.DESKTOP, k.authTag(Secure.Role.DESKTOP))
        assertArrayEquals("hola".toByteArray(), kp.ciphers(Secure.Role.PHONE).second.open(header, ct))
    }

    @Test
    fun hkdfMatchesRfc5869Case1() {
        val okm = Secure.hkdf(
            Secure.unhex("000102030405060708090a0b0c")!!,
            ByteArray(22) { 0x0b },
            Secure.unhex("f0f1f2f3f4f5f6f7f8f9")!!,
            42,
        )
        assertEquals(
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865",
            hex(okm),
        )
    }

    @Test
    fun wrongCodeFailsAndTagsAreRoleBound() {
        val d = Secure.generateKeyPair()
        val p = Secure.generateKeyPair()
        val kd = Secure.derive(d, Secure.publicBytes(p), Secure.codePsk("123456"), ByteArray(0), ByteArray(0))
        val kp = Secure.derive(p, Secure.publicBytes(d), Secure.codePsk("123457"), ByteArray(0), ByteArray(0))
        assertThrows(Secure.AuthFailed::class.java) { kp.verifyAuth(Secure.Role.DESKTOP, kd.authTag(Secure.Role.DESKTOP)) }
        assertThrows(Secure.AuthFailed::class.java) { kd.verifyAuth(Secure.Role.PHONE, kd.authTag(Secure.Role.DESKTOP)) }
    }

    @Test
    fun tamperingReplayAndReorderFail() {
        val key = ByteArray(32) { 7 }
        val tx = Secure.GcmCipher(key)
        val h1 = Opcm.encodeHeader(Opcm.STATS, 0, 0, 0, 3 + Secure.TAG_LEN)
        val h2 = Opcm.encodeHeader(Opcm.STATS, 0, 1, 0, 3 + Secure.TAG_LEN)
        val c1 = tx.seal(h1, "uno".toByteArray())
        val c2 = tx.seal(h2, "dos".toByteArray())
        assertThrows(AEADBadTagException::class.java) { Secure.GcmCipher(key).open(h2, c2) } // reordenado
        val badH = h1.copyOf().also { it[5] = Opcm.CONFIG.toByte() }
        assertThrows(AEADBadTagException::class.java) { Secure.GcmCipher(key).open(badH, c1) } // cabecera alterada
        val rx = Secure.GcmCipher(key)
        assertArrayEquals("uno".toByteArray(), rx.open(h1, c1))
        assertThrows(AEADBadTagException::class.java) { rx.open(h1, c1) } // repetido
    }

    @Test
    fun rejectsInvalidPublicKeys() {
        val kp = Secure.generateKeyPair()
        val params = (kp.public as ECPublicKey).params
        val good = Secure.publicBytes(kp)
        Secure.decodePublic(good, params)
        for (bad in listOf(ByteArray(0), ByteArray(65) { 4 }, good.copyOf(33), good.copyOf().also { it[64] = (it[64].toInt() xor 1).toByte() })) {
            assertThrows(IllegalArgumentException::class.java) { Secure.decodePublic(bad, params) }
        }
    }

    @Test
    fun codesAndHex() {
        assertEquals("123456", Secure.normalizeCode(" 123-456 "))
        for (bad in listOf("12345", "1234567", "12a456", "")) assertNull(Secure.normalizeCode(bad))
        assertArrayEquals(Secure.codePsk("123 456"), Secure.codePsk("123456"))
        repeat(50) { assertEquals(6, Secure.newCode().length) }
        assertEquals("00ab10", Secure.hex(byteArrayOf(0, 0xab.toByte(), 0x10)))
        assertArrayEquals(byteArrayOf(0, 0xab.toByte(), 0x10), Secure.unhex("00ab10"))
        assertNull(Secure.unhex("0"))
        assertNull(Secure.unhex("zz"))
        assertEquals(true, Secure.isId("c0ffee00"))
        assertEquals(false, Secure.isId("c0f"))
        assertEquals(false, Secure.isId("c0ffee0\n"))
    }

    @Test
    fun pairingRotatesCodeOnSuccessAndAfterFailures() {
        val keys = MemoryPairingKeys()
        val p = Pairing("a1b2c3d4", keys)
        var changes = 0
        p.onCodeChanged = { changes++ }
        repeat(Secure.MAX_CODE_ATTEMPTS - 1) { p.onFailure() }
        assertEquals(0, changes)
        p.onFailure()
        assertEquals(1, changes)
        p.onPaired("c0ffee00", ByteArray(32) { 1 })
        assertEquals(2, changes)
        assertArrayEquals(ByteArray(32) { 1 }, keys.get("c0ffee00"))
        p.forgetAll()
        assertNull(keys.get("c0ffee00"))
        assertNotEquals(0, changes)
    }
}
