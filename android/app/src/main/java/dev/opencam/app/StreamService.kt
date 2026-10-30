package dev.opencam.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.Size
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
    private var cameraThread: HandlerThread? = null
    private var cameraHandler: Handler? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var encoder: H264Encoder? = null
    private var server: OpcmServer? = null

    private var config = StreamConfig()

    override fun onCreate() {
        super.onCreate()
        cameraManager = getSystemService(Context.CAMERA_SERVICE) as CameraManager
        cameraThread = HandlerThread("opencam-camera").also { it.start() }
        cameraHandler = Handler(cameraThread!!.looper)
        startForegroundCompat()
        server = OpcmServer(
            capsJson = { capsJson() },
            onConfig = { json -> applyConfig(json) },
            onKeyframeRequest = { encoder?.requestKeyframe() },
            onClientChanged = { connected -> ConnectionState.set(connected) },
        ).also { it.start() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startCapture()
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        stopCapture()
        server?.stop()
        cameraThread?.quitSafely()
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
        config = StreamConfig.parse(json, config)
        // Reconfigurar cámara y codificador con la nueva configuración.
        cameraHandler?.post {
            stopCapture()
            startCapture()
        }
    }

    private fun startCapture() {
        cameraHandler?.post { openCamera() }
    }

    private fun openCamera() {
        val id = findCameraId() ?: return
        val size = Size(config.width, config.height)

        val enc = H264Encoder(size.width, size.height, config.fps, config.bitrateKbps * 1000) { data, ts, key ->
            server?.sendVideo(data, ts, key)
        }
        enc.start()
        encoder = enc

        cameraManager.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(camera: CameraDevice) {
                device = camera
                createSession(camera, enc.inputSurface)
            }

            override fun onDisconnected(camera: CameraDevice) {
                camera.close()
                device = null
            }

            override fun onError(camera: CameraDevice, error: Int) {
                camera.close()
                device = null
            }
        }, cameraHandler)
    }

    private fun createSession(camera: CameraDevice, surface: Surface) {
        camera.createCaptureSession(listOf(surface), object : CameraCaptureSession.StateCallback() {
            override fun onConfigured(s: CameraCaptureSession) {
                session = s
                val request = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                    addTarget(surface)
                    set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
                    set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, android.util.Range(config.fps, config.fps))
                }.build()
                s.setRepeatingRequest(request, null, cameraHandler)
            }

            override fun onConfigureFailed(s: CameraCaptureSession) = Unit
        }, cameraHandler)
    }

    private fun stopCapture() {
        session?.close()
        session = null
        device?.close()
        device = null
        encoder?.stop()
        encoder = null
    }

    /** Fase 1: solo cámara trasera principal (lente "wide"). */
    private fun findCameraId(): String? =
        cameraManager.cameraIdList.firstOrNull { id ->
            cameraManager.getCameraCharacteristics(id).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK
        }

    private fun capsJson(): String = JSONObject()
        .put("platform", "android")
        .put("codecs", JSONArray().put("h264"))
        .put("resolutions", JSONArray().put(JSONArray().put(1920).put(1080)))
        .put("fps", JSONArray().put(30))
        .put("lenses", JSONArray().put("wide"))
        .toString()
}

/** Parámetros de transmisión negociados con el escritorio (CONFIG). */
data class StreamConfig(
    val width: Int = 1920,
    val height: Int = 1080,
    val fps: Int = 30,
    val bitrateKbps: Int = 8000,
) {
    companion object {
        fun parse(json: String, fallback: StreamConfig): StreamConfig {
            val o = JSONObject(json)
            return StreamConfig(
                width = o.optInt("width", fallback.width),
                height = o.optInt("height", fallback.height),
                fps = o.optInt("fps", fallback.fps),
                bitrateKbps = o.optInt("bitrate_kbps", fallback.bitrateKbps),
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
