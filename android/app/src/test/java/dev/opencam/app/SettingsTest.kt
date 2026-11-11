package dev.opencam.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsTest {
    @Test
    fun offersSixteenByNineFrom480p() {
        assertTrue(Caps.isOffered(1920, 1080))
        assertTrue(Caps.isOffered(3840, 2160))
        assertFalse(Caps.isOffered(640, 360))
        assertFalse(Caps.isOffered(1440, 1080))
    }

    @Test
    fun networkKinds() {
        assertEquals("Wi-Fi", NetInfo.kindOf("wlan0"))
        assertEquals("Punto de acceso", NetInfo.kindOf("ap0"))
        assertEquals("USB (anclaje)", NetInfo.kindOf("rndis0"))
        assertEquals("", NetInfo.kindOf("rmnet_data0"))
    }

    @Test
    fun bitrateOptionsAreSorted() {
        val kbps = AppSettings.BITRATES.map { it.second }
        assertEquals(kbps.sorted(), kbps)
        assertTrue(8000 in kbps) // el valor por defecto de StreamConfig
    }
}
