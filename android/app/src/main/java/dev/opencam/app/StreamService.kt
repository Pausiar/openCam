package dev.opencam.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.hardware.camera2.CameraAccessException
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodec
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.Log
import android.util.Range
import android.util.Size
import android.view.Surface
import androidx.core.app.NotificationCompat
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.hypot

/**
 * Servicio en primer plano: mantiene la cámara y el servidor OPCM activos
 * mientras la app está en segundo plano.
 */
class StreamService : Service() {

    private lateinit var cameraManager: CameraManager
    private var cameraThread: HandlerThread? = null
    private var cameraHandler: Handler? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var encoder: H264Encoder? = null
    private var server: OpcmServer? = null
    private var nsd: NsdAdvertiser? = null

    // Los tres solo se tocan en el hilo de cámara.
    private var config = StreamConfig()
    private var active: StreamConfig? = null // configuración con la que corre la captura

    override fun onCreate() {
        super.onCreate()
        cameraManager = getSystemService(Context.CAMERA_SERVICE) as CameraManager
        cameraThread = HandlerThread("opencam-camera").also { it.start() }
        cameraHandler = Handler(cameraThread!!.looper)
        startForegroundCompat()
        server = OpcmServer(
            capsJson = { capsJson() },
            onConfig = { json -> applyConfig(json) },
            onStats = { json -> applyStats(json) },
            onKeyframeRequest = { encoder?.requestKeyframe() },
            onClientChanged = { connected -> ConnectionState.set(connected) },
        ).also { it.start() }
        nsd = NsdAdvertiser(this).also { it.start() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startCapture()
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        nsd?.stop()
        server?.stop()
        // El estado de captura es del hilo de cámara: se libera allí y después se cierra el hilo.
        cameraHandler?.post { stopCapture() }
        cameraThread?.quitSafely()
        ConnectionState.running = false
        ConnectionState.set(false)
        super.onDestroy()
    }

    private fun startForegroundCompat() {
        val channelId = "opencam_stream"
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(channelId, "Transmisión", NotificationManager.IMPORTANCE_LOW))
        val notification: Notification = NotificationCompat.Builder(this, channelId)
            .setContentTitle("OpenCam activo")
            .setContentText("Transmitiendo la cámara al ordenador")
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA)
        } else {
            startForeground(1, notification)
        }
    }

    private fun applyConfig(json: String) {
        cameraHandler?.post {
            val new = try {
                StreamConfig.parse(json, config)
            } catch (e: Exception) {
                server?.sendError("bad_config", "CONFIG inválido")
                return@post
            }
            val old = active
            config = new
            when {
                old == null || encoder == null -> { stopCapture(); openCamera() }
                new == old -> Unit // sin cambios: no se toca cámara ni codificador
                new.copy(bitrateKbps = old.bitrateKbps) == old -> setBitrate(new.bitrateKbps)
                else -> { stopCapture(); openCamera() }
            }
        }
    }

    private fun applyStats(json: String) {
        val kbps = try { JSONObject(json).optInt("target_bitrate_kbps", 0) } catch (e: Exception) { 0 }
        if (kbps > 0) cameraHandler?.post { setBitrate(kbps.coerceIn(MIN_KBPS, MAX_KBPS)) }
    }

    private fun setBitrate(kbps: Int) {
        val enc = encoder ?: return
        if (kbps == active?.bitrateKbps) return
        enc.setBitrate(kbps * 1000)
        active = active?.copy(bitrateKbps = kbps)
        config = config.copy(bitrateKbps = kbps)
    }

    private fun startCapture() {
        cameraHandler?.post { openCamera() }
    }

    private fun fail(code: String, message: String) {
        Log.e(TAG, "$code: $message")
        server?.sendError(code, message)
    }

    private fun openCamera() {
        if (encoder != null) return // ya activo (p. ej. onStartCommand repetido)
        val cfg = config
        val id = findCameraId(cfg.lens) ?: return fail("camera_error", "no hay cámara")
        val mime = if (cfg.codec == "h265") MediaFormat.MIMETYPE_VIDEO_HEVC else MediaFormat.MIMETYPE_VIDEO_AVC

        val enc = H264Encoder(mime, cfg.width, cfg.height, cfg.fps, cfg.bitrateKbps * 1000) { data, ts, key ->
            server?.sendVideo(data, ts, key)
        }
        try {
            enc.start()
        } catch (e: Exception) {
            enc.stop()
            return fail("encoder_error", e.message ?: "no se pudo iniciar el codificador")
        }
        encoder = enc
        active = cfg

        try {
            cameraManager.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) {
                    if (encoder !== enc) { camera.close(); return } // reconfigurado mientras abría
                    device = camera
                    createSession(camera, id, enc.inputSurface, cfg)
                }

                override fun onDisconnected(camera: CameraDevice) {
                    camera.close()
                    if (encoder === enc) { stopCapture(); fail("camera_error", "cámara desconectada") }
                }

                override fun onError(camera: CameraDevice, error: Int) {
                    camera.close()
                    if (encoder === enc) { stopCapture(); fail("camera_error", "error de cámara $error") }
                }
            }, cameraHandler)
        } catch (e: SecurityException) {
            stopCapture()
            fail("camera_permission", "permiso de cámara denegado")
        } catch (e: CameraAccessException) {
            stopCapture()
            fail("camera_error", e.message ?: "no se pudo abrir la cámara")
        }
    }

    private fun createSession(camera: CameraDevice, id: String, surface: Surface, cfg: StreamConfig) {
        try {
            camera.createCaptureSession(listOf(surface), object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(s: CameraCaptureSession) {
                    if (device !== camera) { s.close(); return }
                    session = s
                    try {
                        val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                            addTarget(surface)
                            set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
                            Caps.pickFpsRange(aeRanges(id), cfg.fps)?.let {
                                set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(it.first, it.second))
                            }
                        }.build()
                        s.setRepeatingRequest(request, null, cameraHandler)
                    } catch (e: Exception) {
                        stopCapture()
                        fail("camera_error", e.message ?: "no se pudo iniciar la captura")
                    }
                }

                override fun onConfigureFailed(s: CameraCaptureSession) {
                    stopCapture()
                    fail("camera_error", "sesión rechazada (resolución o fps no soportados)")
                }
            }, cameraHandler)
        } catch (e: Exception) {
            stopCapture()
            fail("camera_error", e.message ?: "no se pudo crear la sesión")
        }
    }

    private fun stopCapture() {
        session?.close()
        session = null
        device?.close()
        device = null
        encoder?.stop()
        encoder = null
        active = null
    }

    private fun aeRanges(id: String): List<Pair<Int, Int>> =
        cameraManager.getCameraCharacteristics(id)
            .get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.map { it.lower to it.upper } ?: emptyList()

    /** Lente ("wide", "ultrawide", "tele") -> id de cámara trasera, por focal equivalente a 35 mm. */
    private fun backLenses(): Map<String, String> {
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

    private fun findCameraId(lens: String): String? {
        val lenses = backLenses()
        return lenses[lens] ?: lenses["wide"] ?: lenses.values.firstOrNull()
    }

    private fun supportedCodecs(): List<String> {
        val encoders = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter { it.isEncoder }
        return listOf("h264" to MediaFormat.MIMETYPE_VIDEO_AVC, "h265" to MediaFormat.MIMETYPE_VIDEO_HEVC)
            .filter { (_, mime) -> encoders.any { info -> info.supportedTypes.any { it.equals(mime, true) } } }
            .map { it.first }
    }

    // shortcut: resoluciones y fps salen de la cámara "wide"; las otras lentes pueden ofrecer menos.
    private fun capsJson(): String {
        val lenses = backLenses()
        val wideId = lenses["wide"] ?: lenses.values.firstOrNull()
        val sizes = wideId?.let {
            cameraManager.getCameraCharacteristics(it)
                .get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                ?.getOutputSizes(MediaCodec::class.java)
                ?.filter { s -> s.width * 9 == s.height * 16 && s.height >= 720 }
                ?.sortedBy { s -> s.width }
        }.orEmpty().ifEmpty { listOf(Size(1920, 1080)) }
        val fps = wideId?.let { Caps.fpsList(aeRanges(it)) }.orEmpty().ifEmpty { listOf(30) }
        return JSONObject()
            .put("platform", "android")
            .put("codecs", JSONArray(supportedCodecs().ifEmpty { listOf("h264") }))
            .put("resolutions", JSONArray(sizes.map { JSONArray().put(it.width).put(it.height) }))
            .put("fps", JSONArray(fps))
            .put("lenses", JSONArray(lenses.keys.toList().ifEmpty { listOf("wide") }))
            .toString()
    }

    companion object {
        private const val TAG = "OpenCam"
        private const val MIN_KBPS = 250
        private const val MAX_KBPS = 100_000
    }
}

/** Parámetros de transmisión negociados con el escritorio (CONFIG). */
data class StreamConfig(
    val width: Int = 1920,
    val height: Int = 1080,
    val fps: Int = 30,
    val bitrateKbps: Int = 8000,
    val codec: String = "h264",
    val lens: String = "wide",
) {
    companion object {
        fun parse(json: String, fallback: StreamConfig): StreamConfig {
            val o = JSONObject(json)
            return StreamConfig(
                width = o.optInt("width", fallback.width),
                height = o.optInt("height", fallback.height),
                fps = o.optInt("fps", fallback.fps),
                bitrateKbps = o.optInt("bitrate_kbps", fallback.bitrateKbps),
                codec = o.optString("codec", fallback.codec),
                lens = o.optString("lens", fallback.lens),
            )
        }
    }
}

/** Estado de conexión observable por la UI. */
object ConnectionState {
    @Volatile var connected: Boolean = false
        private set

    /** Indica si el usuario ha iniciado la transmisión desde la UI. */
    @Volatile var running: Boolean = false

    var listener: ((Boolean) -> Unit)? = null

    fun set(value: Boolean) {
        connected = value
        listener?.invoke(value)
    }
}
