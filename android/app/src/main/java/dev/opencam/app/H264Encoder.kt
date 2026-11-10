package dev.opencam.app

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Bundle
import android.view.Surface

/**
 * Codificador H.264/H.265 por hardware con entrada Surface (sin copias desde Camera2).
 * Entrega unidades de acceso Annex-B; cada keyframe lleva SPS/PPS delante.
 */
class H264Encoder(
    private val mime: String,
    private val width: Int,
    private val height: Int,
    private val fps: Int,
    private val bitrateBps: Int,
    private val onFrame: (data: ByteArray, timestampUs: Long, keyframe: Boolean) -> Unit,
) {
    private var codec: MediaCodec? = null
    private var thread: Thread? = null
    @Volatile private var csd: ByteArray? = null

    /** Superficie donde Camera2 debe dibujar los frames. */
    lateinit var inputSurface: Surface
        private set

    fun start() {
        // Perfil High si el codificador lo admite; algunos móviles solo aceptan el que traen por defecto.
        val c = try {
            configure(highProfile = true)
        } catch (e: Exception) {
            if (mime != MediaFormat.MIMETYPE_VIDEO_AVC) throw e
            configure(highProfile = false)
        }
        codec = c
        inputSurface = c.createInputSurface()
        c.start()

        thread = Thread({ drainLoop(c) }, "opencam-h264-drain").apply { start() }
    }

    /** Crea y configura el codificador; si falla lo libera y relanza el error. */
    private fun configure(highProfile: Boolean): MediaCodec {
        val format = MediaFormat.createVideoFormat(mime, width, height).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrateBps)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)
            setInteger(MediaFormat.KEY_PRIORITY, 0) // tiempo real
            setInteger(MediaFormat.KEY_LATENCY, 1)
            if (highProfile && mime == MediaFormat.MIMETYPE_VIDEO_AVC) {
                setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
            }
        }
        val c = MediaCodec.createEncoderByType(mime)
        try {
            c.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        } catch (e: Exception) {
            c.release()
            throw e
        }
        return c
    }

    /** Pide un keyframe inmediato (KEYFRAME_REQUEST del escritorio). */
    fun requestKeyframe() {
        runCatching { codec?.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) }) }
    }

    /** Cambia el bitrate en caliente, sin reiniciar el codificador. */
    fun setBitrate(bitrateBps: Int) {
        runCatching { codec?.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_VIDEO_BITRATE, bitrateBps) }) }
    }

    fun stop() {
        val t = thread
        thread = null
        t?.interrupt()
        codec?.let {
            runCatching { it.signalEndOfInputStream() }
            // El hilo de drenaje debe soltar el codificador antes de liberarlo.
            runCatching { t?.join(500) }
            runCatching { it.stop() }
            runCatching { it.release() }
        }
        codec = null
        if (::inputSurface.isInitialized) inputSurface.release()
    }

    private fun drainLoop(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        try {
            while (!Thread.currentThread().isInterrupted) {
                val index = c.dequeueOutputBuffer(info, 10_000)
                if (index < 0) continue // sin salida aún, o cambio de formato
                val buf = c.getOutputBuffer(index)
                if (buf != null && info.size > 0) {
                    buf.position(info.offset)
                    buf.limit(info.offset + info.size)
                    val data = ByteArray(info.size)
                    buf.get(data)
                    if ((info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG) != 0) {
                        // SPS/PPS: se guardan y se anteponen a cada keyframe.
                        csd = data
                    } else {
                        val key = (info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME) != 0
                        val out = if (key && csd != null) csd!! + data else data
                        onFrame(out, info.presentationTimeUs, key)
                    }
                }
                c.releaseOutputBuffer(index, false)
                if ((info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM) != 0) break
            }
        } catch (e: IllegalStateException) {
            // Codificador detenido o liberado mientras se drenaba (stop() desde otro hilo): fin normal.
        }
    }
}
