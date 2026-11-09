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
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.Log
import android.util.Range
import android.view.Surface
import androidx.core.app.NotificationCompat
import org.json.JSONArray
import org.json.JSONObject

/**
 * Servicio en primer plano: mantiene la cámara y el servidor OPCM activos
 * mientras la app está en segundo plano.
 */
class StreamService : Service() {

    private lateinit var cameraManager: CameraManager
    private lateinit var cameras: CameraInfo
    private var cameraThread: HandlerThread? = null
    private var cameraHandler: Handler? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var encoder: H264Encoder? = null
    private var server: OpcmServer? = null
    private var nsd: NsdAdvertiser? = null

    // Ajustes elegidos en la app: base de cada CONFIG (el PC puede pedir solo algunos campos).
    private var local = StreamConfig()
    // Los tres solo se tocan en el hilo de cámara.
    private var config = StreamConfig()
    private var active: StreamConfig? = null // configuración con la que corre la captura

    override fun onCreate() {
        super.onCreate()
        cameraManager = getSystemService(Context.CAMERA_SERVICE) as CameraManager
        cameras = CameraInfo(cameraManager)
        local = AppSettings.load(this)
        config = local
        cameraThread = HandlerThread("opencam-camera").also { it.start() }
        cameraHandler = Handler(cameraThread!!.looper)
        startForegroundCompat()
        server = OpcmServer(
            capsJson = { capsJson() },
            onConfig = { json -> applyConfig(json) },
            onStats = { json -> applyStats(json) },
            onKeyframeRequest = { encoder?.requestKeyframe() },
            onClientChanged = { connected ->
                // Un PC nuevo parte de los ajustes del móvil, no de lo que pidió el anterior.
                if (connected) cameraHandler?.post { config = local }
                ConnectionState.set(connected)
            },
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
                StreamConfig.parse(json, local)
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
        val id = cameras.findCameraId(cfg.lens) ?: return fail("camera_error", "no hay cámara")
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
                            Caps.pickFpsRange(cameras.aeRanges(id), cfg.fps)?.let {
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

    private fun supportedCodecs(): List<String> {
        val encoders = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter { it.isEncoder }
        return listOf("h264" to MediaFormat.MIMETYPE_VIDEO_AVC, "h265" to MediaFormat.MIMETYPE_VIDEO_HEVC)
            .filter { (_, mime) -> encoders.any { info -> info.supportedTypes.any { it.equals(mime, true) } } }
            .map { it.first }
    }

    private fun capsJson(): String {
        val o = cameras.options()
        return JSONObject()
            .put("platform", "android")
            .put("codecs", JSONArray(supportedCodecs().ifEmpty { listOf("h264") }))
            .put("resolutions", JSONArray(o.sizes.map { JSONArray().put(it.width).put(it.height) }))
            .put("fps", JSONArray(o.fps))
            .put("lenses", JSONArray(o.lenses))
            .put("current", local.toJson())
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
    /** Ajustes locales que se anuncian en CAPS como `current`. */
    fun toJson(): JSONObject = JSONObject()
        .put("width", width)
        .put("height", height)
        .put("fps", fps)
        .put("bitrate_kbps", bitrateKbps)
        .put("lens", lens)

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
