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

    /** Resoluciones que se ofrecen: 16:9 desde 480p (el escritorio escala a su salida). */
    fun isOffered(width: Int, height: Int): Boolean = width * 9 == height * 16 && height >= 480

    /** fps ofrecidos: techos de los rangos AE, a partir de 15. */
    fun fpsList(ranges: List<Pair<Int, Int>>): List<Int> =
        ranges.map { it.second }.filter { it >= 15 }.distinct().sorted()
}
