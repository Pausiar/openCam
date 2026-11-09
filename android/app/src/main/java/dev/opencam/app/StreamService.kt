package dev.opencam.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Rect
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
    private var cameraId: String? = null
    private var captureSurface: Surface? = null
    private var server: OpcmServer? = null
    private var nsd: NsdAdvertiser? = null

    // Solo se tocan en el hilo de cámara. Arranca con lo elegido en la app (AdvancedSettings).
    private var config = AdvancedSettings.current
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
        // Cambios desde la pantalla del móvil: mismo camino que un CONFIG del escritorio.
        AdvancedSettings.serviceHandler = { change -> cameraHandler?.post { applyNew(change(config)) } }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startCapture()
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        AdvancedSettings.serviceHandler = null
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
            applyNew(new)
        }
    }

    /** Solo en el hilo de cámara. Zoom, enfoque, estabilización y bitrate se aplican en caliente. */
    private fun applyNew(new: StreamConfig) {
        val old = active
        config = new
        when {
            old == null || encoder == null -> { stopCapture(); openCamera() }
            new == old -> Unit // sin cambios: no se toca cámara ni codificador
            old.onlyLiveChanges(new) -> {
                setBitrate(new.bitrateKbps)
                active = new
                updateRequest()
            }
            else -> { stopCapture(); openCamera() }
        }
        AdvancedSettings.publish(config)
    }

    /** Reemplaza la petición repetida con los ajustes activos, sin cerrar la sesión. */
    private fun updateRequest() {
        val s = session ?: return // aún abriendo: onConfigured usará `active`
        val cam = device ?: return
        val id = cameraId ?: return
        val surface = captureSurface ?: return
        val cfg = active ?: return
        try {
            s.setRepeatingRequest(buildRequest(cam, id, surface, cfg), null, cameraHandler)
        } catch (e: Exception) {
            fail("camera_error", e.message ?: "no se pudo aplicar el ajuste")
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
        AdvancedSettings.publish(config)
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
        val id = findCameraId(cfg) ?: return fail("camera_error", "no hay cámara")
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
        cameraId = id
        captureSurface = enc.inputSurface

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
                        // `active` puede traer zoom/enfoque cambiados mientras se abría la cámara.
                        val request = buildRequest(camera, id, surface, active ?: cfg)
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

    private fun buildRequest(camera: CameraDevice, id: String, surface: Surface, cfg: StreamConfig): CaptureRequest {
        val ch = cameraManager.getCameraCharacteristics(id)
        return camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
            addTarget(surface)
            Caps.pickFpsRange(aeRanges(id), cfg.fps)?.let {
                set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(it.first, it.second))
            }

            // Enfoque: manual solo si la lente lo permite (las de foco fijo dan 0).
            val minFocus = ch.get(CameraCharacteristics.LENS_INFO_MINIMUM_FOCUS_DISTANCE) ?: 0f
            val afModes = ch.get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES) ?: intArrayOf()
            if (cfg.focus == "manual" && minFocus > 0f) {
                set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_OFF)
                set(CaptureRequest.LENS_FOCUS_DISTANCE, Caps.focusDiopters(cfg.focusDistance, minFocus))
            } else if (CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO in afModes) {
                set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
            }

            // Estabilización: electrónica (EIS) y óptica (OIS) si existen.
            val eis = ch.get(CameraCharacteristics.CONTROL_AVAILABLE_VIDEO_STABILIZATION_MODES) ?: intArrayOf()
            if (CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON in eis) {
                set(
                    CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE,
                    if (cfg.stabilization) CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_ON
                    else CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_OFF,
                )
            }
            val ois = ch.get(CameraCharacteristics.LENS_INFO_AVAILABLE_OPTICAL_STABILIZATION) ?: intArrayOf()
            if (CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE_ON in ois) {
                set(
                    CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE,
                    if (cfg.stabilization) CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE_ON
                    else CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE_OFF,
                )
            }

            // Zoom: CONTROL_ZOOM_RATIO en Android 11+, recorte del sensor en versiones anteriores.
            val zoom = Caps.clampZoom(cfg.zoom, maxZoom(id))
            if (Build.VERSION.SDK_INT >= 30) {
                ch.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)?.let {
                    set(CaptureRequest.CONTROL_ZOOM_RATIO, zoom.coerceIn(it.lower, it.upper))
                }
            } else {
                ch.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE)?.let { a ->
                    val (l, t, w, h) = Caps.cropRegion(a.width(), a.height(), zoom).toList()
                    set(CaptureRequest.SCALER_CROP_REGION, Rect(l, t, l + w, t + h))
                }
            }
        }.build()
    }

    private fun maxZoom(id: String): Float {
        val ch = cameraManager.getCameraCharacteristics(id)
        if (Build.VERSION.SDK_INT >= 30) {
            ch.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE)?.let { return it.upper }
        }
        return ch.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM) ?: 1f
    }

    private fun stopCapture() {
        session?.close()
        session = null
        device?.close()
        device = null
        encoder?.stop()
        encoder = null
        active = null
        cameraId = null
        captureSurface = null
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

    private fun frontCameraId(): String? = cameraManager.cameraIdList.firstOrNull {
        cameraManager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) ==
            CameraCharacteristics.LENS_FACING_FRONT
    }

    /** Frontal si se pide y existe; si no, la lente trasera pedida (o la principal). */
    private fun findCameraId(cfg: StreamConfig): String? {
        if (cfg.facing == "front") frontCameraId()?.let { return it }
        val lenses = backLenses()
        return lenses[cfg.lens] ?: lenses["wide"] ?: lenses.values.firstOrNull() ?: frontCameraId()
    }

    private fun hasStabilization(id: String): Boolean {
        val ch = cameraManager.getCameraCharacteristics(id)
        val eis = ch.get(CameraCharacteristics.CONTROL_AVAILABLE_VIDEO_STABILIZATION_MODES) ?: intArrayOf()
        val ois = ch.get(CameraCharacteristics.LENS_INFO_AVAILABLE_OPTICAL_STABILIZATION) ?: intArrayOf()
        return CameraCharacteristics.CONTROL_VIDEO_STABILIZATION_MODE_ON in eis ||
            CameraCharacteristics.LENS_OPTICAL_STABILIZATION_MODE_ON in ois
    }

    private fun hasManualFocus(id: String): Boolean =
        (cameraManager.getCameraCharacteristics(id).get(CameraCharacteristics.LENS_INFO_MINIMUM_FOCUS_DISTANCE) ?: 0f) > 0f

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
            .put("facings", JSONArray(listOfNotNull(wideId?.let { "back" }, frontCameraId()?.let { "front" })))
            .put("max_zoom", wideId?.let { maxZoom(it).toDouble() } ?: 1.0)
            .put("stabilization", wideId?.let { hasStabilization(it) } ?: false)
            .put("manual_focus", wideId?.let { hasManualFocus(it) } ?: false)
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
    /** "back" o "front". */
    val facing: String = "back",
    val stabilization: Boolean = true,
    /** 1.0 = sin zoom; se limita al máximo de la cámara. */
    val zoom: Float = 1f,
    /** "auto" (continuo) o "manual". */
    val focus: String = "auto",
    /** Enfoque manual: 0 = infinito, 1 = lo más cerca posible. */
    val focusDistance: Float = 0f,
) {
    /** True si `new` solo cambia ajustes que se aplican sin reiniciar cámara ni codificador. */
    fun onlyLiveChanges(new: StreamConfig): Boolean =
        new.copy(
            bitrateKbps = bitrateKbps,
            stabilization = stabilization,
            zoom = zoom,
            focus = focus,
            focusDistance = focusDistance,
        ) == this

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
                facing = o.optString("facing", fallback.facing).takeIf { it == "front" || it == "back" } ?: fallback.facing,
                stabilization = o.optBoolean("stabilization", fallback.stabilization),
                zoom = o.optDouble("zoom", fallback.zoom.toDouble()).toFloat().takeIf { it.isFinite() } ?: fallback.zoom,
                focus = o.optString("focus", fallback.focus).takeIf { it == "auto" || it == "manual" } ?: fallback.focus,
                focusDistance = o.optDouble("focus_distance", fallback.focusDistance.toDouble()).toFloat()
                    .takeIf { it.isFinite() }?.coerceIn(0f, 1f) ?: fallback.focusDistance,
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
