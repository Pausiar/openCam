package dev.opencam.app

import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.media.MediaCodec
import android.util.Size
import kotlin.math.hypot

/** Lo que ofrecen las cámaras traseras; lo usan el servicio (CAPS) y la pantalla de ajustes. */
class CameraInfo(private val cameraManager: CameraManager) {

    /** Lente ("wide", "ultrawide", "tele") -> id de cámara trasera, por focal equivalente a 35 mm. */
    fun backLenses(): Map<String, String> {
        val out = LinkedHashMap<String, String>()
        for (id in cameraManager.cameraIdList) {
            val c = cameraManager.getCameraCharacteristics(id)
            if (c.get(CameraCharacteristics.LENS_FACING) != CameraCharacteristics.LENS_FACING_BACK) continue
            val f = c.get(CameraCharacteristics.LENS_INFO_AVAILABLE_FOCAL_LENGTHS)?.firstOrNull()
            val sz = c.get(CameraCharacteristics.SENSOR_INFO_PHYSICAL_SIZE)
            val name = if (f != null && sz != null) {
                Caps.lensName(f * 43.27f / hypot(sz.width, sz.height))
            } else "wide"
            out.putIfAbsent(name, id)
        }
        return out
    }

    fun findCameraId(lens: String): String? {
        val lenses = backLenses()
        return lenses[lens] ?: lenses["wide"] ?: lenses.values.firstOrNull()
    }

    fun aeRanges(id: String): List<Pair<Int, Int>> =
        cameraManager.getCameraCharacteristics(id)
            .get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.map { it.lower to it.upper } ?: emptyList()

    /** Tamaños 16:9 que el codificador puede recibir de la cámara, de menor a mayor. */
    fun sizes(id: String): List<Size> =
        cameraManager.getCameraCharacteristics(id)
            .get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?.getOutputSizes(MediaCodec::class.java)
            ?.filter { s -> Caps.isOffered(s.width, s.height) }
            ?.distinct()
            ?.sortedBy { s -> s.width }
            .orEmpty()

    // shortcut: resoluciones y fps salen de la cámara "wide"; las otras lentes pueden ofrecer menos.
    fun options(): Options {
        val lenses = runCatching { backLenses() }.getOrDefault(emptyMap())
        val id = lenses["wide"] ?: lenses.values.firstOrNull()
        val sizes = id?.let { runCatching { sizes(it) }.getOrNull() }.orEmpty().ifEmpty { listOf(Size(1920, 1080)) }
        val fps = id?.let { runCatching { Caps.fpsList(aeRanges(it)) }.getOrNull() }.orEmpty().ifEmpty { listOf(30) }
        return Options(sizes, fps, lenses.keys.toList().ifEmpty { listOf("wide") })
    }

    data class Options(val sizes: List<Size>, val fps: List<Int>, val lenses: List<String>)
}
