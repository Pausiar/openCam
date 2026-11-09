package dev.opencam.app

import kotlin.math.abs

/** Lógica pura (sin APIs de Android) para CAPS y selección de rangos; probada en JVM. */
object Caps {
    /** Nombre de lente según la focal equivalente a 35 mm. */
    fun lensName(focal35mm: Float): String = when {
        focal35mm < 20f -> "ultrawide"
        focal35mm >= 45f -> "tele"
        else -> "wide"
    }

    /** Rango AE (lower, upper) más estrecho que contenga `fps`; si ninguno, el de techo más cercano. */
    fun pickFpsRange(ranges: List<Pair<Int, Int>>, fps: Int): Pair<Int, Int>? =
        ranges.filter { fps in it.first..it.second }.minByOrNull { it.second - it.first }
            ?: ranges.minByOrNull { abs(it.second - fps) }

    /** fps ofrecidos: techos de los rangos AE, a partir de 15. */
    fun fpsList(ranges: List<Pair<Int, Int>>): List<Int> =
        ranges.map { it.second }.filter { it >= 15 }.distinct().sorted()

    /** Zoom pedido limitado a [1, maxZoom] (maxZoom < 1 se trata como 1). */
    fun clampZoom(zoom: Float, maxZoom: Float): Float =
        if (zoom.isNaN()) 1f else zoom.coerceIn(1f, maxOf(1f, maxZoom))

    /**
     * Recorte centrado para zoom digital (dispositivos sin CONTROL_ZOOM_RATIO, Android < 11).
     * Devuelve (left, top, width, height) dentro de la matriz activa del sensor.
     */
    fun cropRegion(activeW: Int, activeH: Int, zoom: Float): IntArray {
        val z = maxOf(1f, zoom)
        val w = (activeW / z).toInt().coerceIn(1, activeW)
        val h = (activeH / z).toInt().coerceIn(1, activeH)
        return intArrayOf((activeW - w) / 2, (activeH - h) / 2, w, h)
    }

    /**
     * Enfoque manual normalizado (0 = infinito, 1 = lo más cerca) a dioptrías de Camera2.
     * `minFocusDiopters` es LENS_INFO_MINIMUM_FOCUS_DISTANCE (0 = foco fijo).
     */
    fun focusDiopters(normalized: Float, minFocusDiopters: Float): Float =
        if (normalized.isNaN()) 0f else normalized.coerceIn(0f, 1f) * maxOf(0f, minFocusDiopters)
}
