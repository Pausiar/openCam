package dev.opencam.app

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CapsTest {
    @Test
    fun zoomIsClamped() {
        assertEquals(1f, Caps.clampZoom(0.5f, 8f), 0f)
        assertEquals(8f, Caps.clampZoom(20f, 8f), 0f)
        assertEquals(2.5f, Caps.clampZoom(2.5f, 8f), 0f)
        assertEquals(1f, Caps.clampZoom(3f, 0f), 0f) // sin zoom disponible
        assertEquals(1f, Caps.clampZoom(Float.NaN, 8f), 0f)
    }

    @Test
    fun cropRegionIsCentered() {
        assertArrayEquals(intArrayOf(0, 0, 4000, 3000), Caps.cropRegion(4000, 3000, 1f))
        assertArrayEquals(intArrayOf(1000, 750, 2000, 1500), Caps.cropRegion(4000, 3000, 2f))
        assertArrayEquals(intArrayOf(0, 0, 4000, 3000), Caps.cropRegion(4000, 3000, 0.3f))
    }

    @Test
    fun focusMapsToDiopters() {
        assertEquals(0f, Caps.focusDiopters(0f, 10f), 0f)
        assertEquals(5f, Caps.focusDiopters(0.5f, 10f), 1e-6f)
        assertEquals(10f, Caps.focusDiopters(2f, 10f), 0f)
        assertEquals(0f, Caps.focusDiopters(0.7f, 0f), 0f) // foco fijo
    }

    @Test
    fun liveChangesDoNotRestartCamera() {
        val base = StreamConfig()
        assertTrue(base.onlyLiveChanges(base.copy(zoom = 2f, stabilization = false, bitrateKbps = 4000)))
        assertTrue(base.onlyLiveChanges(base.copy(focus = "manual", focusDistance = 0.3f)))
        assertFalse(base.onlyLiveChanges(base.copy(facing = "front")))
        assertFalse(base.onlyLiveChanges(base.copy(width = 1280, height = 720)))
    }
}
