package dev.opencam.app

import android.content.Context

/** Ajustes de vídeo elegidos en el móvil. El PC puede sustituirlos con CONFIG o respetarlos. */
object AppSettings {
    private const val PREFS = "opencam"

    fun load(context: Context): StreamConfig {
        val p = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val d = StreamConfig()
        return StreamConfig(
            width = p.getInt("width", d.width),
            height = p.getInt("height", d.height),
            fps = p.getInt("fps", d.fps),
            bitrateKbps = p.getInt("bitrate_kbps", d.bitrateKbps),
            lens = p.getString("lens", d.lens) ?: d.lens,
        )
    }

    fun save(context: Context, c: StreamConfig) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putInt("width", c.width)
            .putInt("height", c.height)
            .putInt("fps", c.fps)
            .putInt("bitrate_kbps", c.bitrateKbps)
            .putString("lens", c.lens)
            .apply()
    }

    /** Calidades ofrecidas en la app: (etiqueta, kbps). */
    val BITRATES = listOf(
        "Baja (2 Mbps)" to 2000,
        "Media (4 Mbps)" to 4000,
        "Alta (8 Mbps)" to 8000,
        "Muy alta (12 Mbps)" to 12000,
        "Máxima (20 Mbps)" to 20000,
    )

    val LENS_NAMES = mapOf("wide" to "Principal", "ultrawide" to "Gran angular", "tele" to "Teleobjetivo")
}
