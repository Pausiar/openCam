package dev.opencam.app

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.hardware.camera2.CameraManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.Spinner
import android.widget.TextView
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var toggle: Button
    private lateinit var wifi: TextView
    private lateinit var device: TextView
    private lateinit var usb: TextView
    private lateinit var resolution: Spinner
    private lateinit var fps: Spinner
    private lateinit var bitrate: Spinner
    private lateinit var lens: Spinner
    private lateinit var advanced: AdvancedControls

    private lateinit var options: CameraInfo.Options
    private var settings = StreamConfig()

    private val running get() = ConnectionState.running

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        status = findViewById(R.id.status)
        toggle = findViewById(R.id.toggle)
        wifi = findViewById(R.id.wifi)
        device = findViewById(R.id.device)
        usb = findViewById(R.id.usb)
        resolution = findViewById(R.id.resolution)
        fps = findViewById(R.id.fps)
        bitrate = findViewById(R.id.bitrate)
        lens = findViewById(R.id.lens)
        advanced = AdvancedControls(findViewById(R.id.advanced))

        settings = AppSettings.load(this)
        AdvancedSettings.publish(AppSettings.withBasics(AdvancedSettings.current, settings))
        options = CameraInfo(getSystemService(Context.CAMERA_SERVICE) as CameraManager).options()
        setupSettings()

        toggle.setOnClickListener {
            if (ConnectionState.running) stopStream() else requestPermissionsAndStart()
        }
    }

    override fun onResume() {
        super.onResume()
        ConnectionState.listener = { runOnUiThread { refresh() } }
        advanced.onResume { runOnUiThread(it) }
        refresh()
    }

    override fun onPause() {
        ConnectionState.listener = null
        advanced.onPause()
        super.onPause()
    }

    private fun <T> bind(spinner: Spinner, items: List<T>, label: (T) -> String, selected: Int, onPick: (T) -> Unit) {
        spinner.adapter = ArrayAdapter(this, android.R.layout.simple_spinner_item, items.map(label)).also {
            it.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item)
        }
        spinner.setSelection(selected.coerceIn(0, (items.size - 1).coerceAtLeast(0)), false)
        spinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                items.getOrNull(position)?.let(onPick)
                AppSettings.save(this@MainActivity, settings)
                if (!running) AdvancedSettings.publish(AppSettings.withBasics(AdvancedSettings.current, settings))
            }

            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
    }

    private fun setupSettings() {
        val sizes = options.sizes
        val sizeIdx = sizes.indexOfFirst { it.width == settings.width && it.height == settings.height }
            .takeIf { it >= 0 } ?: sizes.indexOfLast { it.height <= 1080 }.coerceAtLeast(0)
        bind(resolution, sizes, { "${it.width}x${it.height}" + if (it.height >= 2160) " (4K)" else "" }, sizeIdx) {
            settings = settings.copy(width = it.width, height = it.height)
        }

        val fpsIdx = options.fps.indexOf(settings.fps).takeIf { it >= 0 }
            ?: options.fps.indexOfLast { it <= 30 }.coerceAtLeast(0)
        bind(fps, options.fps, { "$it fps" }, fpsIdx) { settings = settings.copy(fps = it) }

        val rates = AppSettings.BITRATES
        val rateIdx = rates.indexOfFirst { it.second == settings.bitrateKbps }.takeIf { it >= 0 }
            ?: rates.indexOfFirst { it.second >= settings.bitrateKbps }.takeIf { it >= 0 } ?: 2
        bind(bitrate, rates, { it.first }, rateIdx) { settings = settings.copy(bitrateKbps = it.second) }

        val lensIdx = options.lenses.indexOf(settings.lens).coerceAtLeast(0)
        bind(lens, options.lenses, { AppSettings.LENS_NAMES[it] ?: it }, lensIdx) { settings = settings.copy(lens = it) }
    }

    private fun requestPermissionsAndStart() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            startStream()
        } else {
            // POST_NOTIFICATIONS (Android 13+): sin él el aviso del servicio queda oculto, pero la cámara funciona.
            val perms = if (Build.VERSION.SDK_INT >= 33) {
                arrayOf(Manifest.permission.CAMERA, Manifest.permission.POST_NOTIFICATIONS)
            } else arrayOf(Manifest.permission.CAMERA)
            ActivityCompat.requestPermissions(this, perms, REQ_CAMERA)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != REQ_CAMERA) return
        val cam = permissions.indexOf(Manifest.permission.CAMERA)
        if (cam >= 0 && grantResults.getOrNull(cam) == PackageManager.PERMISSION_GRANTED) {
            startStream()
        } else {
            status.text = "Permiso de cámara denegado"
        }
    }

    private fun startStream() {
        AppSettings.save(this, settings)
        // El servicio arranca con AdvancedSettings.current: calidad de aquí y zoom/enfoque de la sección avanzada.
        AdvancedSettings.publish(AppSettings.withBasics(AdvancedSettings.current, settings))
        ContextCompat.startForegroundService(this, Intent(this, StreamService::class.java))
        ConnectionState.running = true
        refresh()
    }

    private fun stopStream() {
        stopService(Intent(this, StreamService::class.java))
        ConnectionState.running = false
        refresh()
    }

    private fun refresh() {
        toggle.text = if (running) "Detener" else "Iniciar"
        status.text = when {
            !running -> "Parado. Pulsa Iniciar y abre OpenCam en el ordenador: " +
                "te encontrará solo por USB (con depuración USB) o por la Wi-Fi."
            ConnectionState.connected -> "Conectado al ordenador"
            else -> "Esperando al ordenador…"
        }
        listOf(resolution, fps, bitrate, lens).forEach { it.isEnabled = !running }

        val port = Opcm.DEFAULT_PORT
        val addrs = NetInfo.addresses(this)
        wifi.text = if (addrs.isEmpty()) {
            "Sin Wi-Fi. Conéctate a la misma red que el PC o usa USB."
        } else {
            addrs.joinToString("\n") { "${it.ip}:$port  (${it.kind})" }
        }
        device.text = "Nombre del móvil: ${deviceName()}. Si el PC no lo encuentra solo, " +
            "escribe la dirección de arriba en «Wi-Fi»."
        usb.text = "Por USB: activa la depuración USB en el móvil y conecta el cable; el PC hace el " +
            "reenvío del puerto $port automáticamente (necesita adb)."
    }

    private fun deviceName(): String =
        Settings.Global.getString(contentResolver, "device_name")?.takeIf { it.isNotBlank() } ?: Build.MODEL

    companion object {
        private const val REQ_CAMERA = 1
    }
}
