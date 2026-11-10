package dev.opencam.app

import android.Manifest
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
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
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.Spinner
import android.widget.TextView
import android.widget.Toast
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var toggle: Button
    private lateinit var wifi: TextView
    private lateinit var device: TextView
    private lateinit var usb: TextView
    private lateinit var pairing: TextView
    private lateinit var pairingLabel: TextView
    private lateinit var forget: Button
    private lateinit var resolution: Spinner
    private lateinit var fps: Spinner
    private lateinit var bitrate: Spinner
    private lateinit var lens: Spinner
    private lateinit var advanced: AdvancedControls
    private lateinit var statusShort: TextView
    private lateinit var statusDot: View
    private lateinit var statusIcon: ImageView
    private lateinit var facingText: TextView
    private lateinit var infoChip: TextView
    private lateinit var wifiAll: TextView
    private lateinit var pages: Map<Int, View>
    private var page = R.id.page_home

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
        pairing = findViewById(R.id.pairing)
        pairingLabel = findViewById(R.id.pairing_label)
        forget = findViewById(R.id.forget)
        forget.setOnClickListener {
            PrefsPairingKeys(applicationContext).clear()
            Toast.makeText(this, "Ordenadores olvidados: tendrán que usar el código otra vez", Toast.LENGTH_LONG).show()
        }
        resolution = findViewById(R.id.resolution)
        fps = findViewById(R.id.fps)
        bitrate = findViewById(R.id.bitrate)
        lens = findViewById(R.id.lens)
        statusShort = findViewById(R.id.status_short)
        statusDot = findViewById(R.id.status_dot)
        statusIcon = findViewById(R.id.status_icon)
        facingText = findViewById(R.id.facing_text)
        infoChip = findViewById(R.id.info_chip)
        wifiAll = findViewById(R.id.wifi_all)
        advanced = AdvancedControls(findViewById(R.id.advanced)) { renderCamera(it) }

        setupNavigation()
        val flipCamera = View.OnClickListener { _ ->
            AdvancedSettings.change { c -> c.copy(facing = if (c.facing == "front") "back" else "front") }
        }
        findViewById<View>(R.id.facing_chip).setOnClickListener(flipCamera)
        findViewById<View>(R.id.switch_camera).setOnClickListener(flipCamera)
        findViewById<View>(R.id.copy).setOnClickListener { copyAddress() }
        findViewById<View>(R.id.open_advanced).setOnClickListener { showPage(R.id.page_settings) }
        findViewById<TextView>(R.id.version).text =
            "OpenCam ${appVersion()} · código abierto (licencia MIT)"

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
        spinner.adapter = ArrayAdapter(this, R.layout.spinner_item, items.map(label)).also {
            it.setDropDownViewResource(R.layout.spinner_dropdown_item)
        }
        spinner.setSelection(selected.coerceIn(0, (items.size - 1).coerceAtLeast(0)), false)
        spinner.onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
            override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                items.getOrNull(position)?.let(onPick)
                AppSettings.save(this@MainActivity, settings)
                if (!running) AdvancedSettings.publish(AppSettings.withBasics(AdvancedSettings.current, settings))
                renderCamera(AdvancedSettings.current)
            }

            override fun onNothingSelected(parent: AdapterView<*>?) = Unit
        }
    }

    private fun setupSettings() {
        val sizes = options.sizes
        val sizeIdx = sizes.indexOfFirst { it.width == settings.width && it.height == settings.height }
            .takeIf { it >= 0 } ?: sizes.indexOfLast { it.height <= 1080 }.coerceAtLeast(0)
        bind(resolution, sizes, { "${it.width} × ${it.height}" + if (it.height >= 2160) " (4K)" else "" }, sizeIdx) {
            settings = settings.copy(width = it.width, height = it.height)
        }

        val fpsIdx = options.fps.indexOf(settings.fps).takeIf { it >= 0 }
            ?: options.fps.indexOfLast { it <= 30 }.coerceAtLeast(0)
        bind(fps, options.fps, { "$it" }, fpsIdx) { settings = settings.copy(fps = it) }

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
        toggle.text = if (running) "Detener servidor" else "Iniciar servidor"
        toggle.setCompoundDrawablesRelativeWithIntrinsicBounds(
            if (running) R.drawable.ic_stop else R.drawable.ic_play, 0, 0, 0,
        )
        val connected = running && ConnectionState.connected
        statusShort.text = when {
            !running -> "Servidor parado"
            connected -> "Conectado al PC"
            else -> "Servidor activo"
        }
        statusDot.background.mutate().setTint(getColor(if (running) R.color.green else R.color.gray_dot))
        status.text = when {
            !running -> "Pulsa «Iniciar servidor» y abre OpenCam en el ordenador: " +
                "te encontrará solo por USB (con depuración USB) o por la Wi-Fi."
            connected -> "Transmitiendo al ordenador"
            else -> "Esperando al ordenador…"
        }
        statusIcon.setImageResource(if (connected) R.drawable.ic_videocam else R.drawable.ic_photo_camera)
        listOf(resolution, fps, bitrate, lens).forEach { it.isEnabled = !running; it.alpha = if (running) 0.6f else 1f }
        forget.isEnabled = !running

        // Código de emparejamiento: el PC lo pide solo la primera vez.
        // Siempre visible: parado avisa de que sale al iniciar; en marcha muestra el código
        // (también conectado, para emparejar otro PC).
        val code = ConnectionState.pairingCode?.takeIf { running }
        pairingLabel.visibility = View.VISIBLE
        pairing.visibility = if (code != null) View.VISIBLE else View.GONE
        pairing.text = code?.let { "${it.take(3)} ${it.drop(3)}" } ?: ""
        pairingLabel.text = when {
            !running -> "El código de emparejamiento aparece aquí al pulsar Iniciar."
            code == null -> "Preparando el código de emparejamiento…"
            else -> "Código de emparejamiento (escríbelo en el PC la primera vez)"
        }

        val port = Opcm.DEFAULT_PORT
        val addrs = NetInfo.addresses(this)
        wifi.text = addrs.firstOrNull()?.let { "${it.ip}:$port" } ?: "Sin Wi-Fi"
        wifiAll.text = if (addrs.isEmpty()) {
            "Sin Wi-Fi. Conéctate a la misma red que el PC o usa USB."
        } else {
            addrs.joinToString("\n") { "${it.ip}:$port  (${it.kind})" }
        }
        device.text = "Nombre del móvil: ${deviceName()}. Si el PC no lo encuentra solo, " +
            "escribe una de estas direcciones en «IP manual»."
        usb.text = "Activa la depuración USB en el móvil y conecta el cable; el PC hace el " +
            "reenvío del puerto $port automáticamente (necesita adb)."
        renderCamera(AdvancedSettings.current)
    }

    /** Etiquetas sobre la tarjeta de la cámara: lente/cara y calidad en uso. */
    private fun renderCamera(cfg: StreamConfig) {
        facingText.text = if (cfg.facing == "front") "Cámara frontal" else "Cámara trasera"
        infoChip.text = "${cfg.width} × ${cfg.height} · ${cfg.fps} FPS"
    }

    private fun setupNavigation() {
        pages = mapOf(
            R.id.page_home to findViewById(R.id.page_home),
            R.id.page_conn to findViewById(R.id.page_conn),
            R.id.page_settings to findViewById(R.id.page_settings),
        )
        findViewById<View>(R.id.nav_home).setOnClickListener { showPage(R.id.page_home) }
        findViewById<View>(R.id.nav_conn).setOnClickListener { showPage(R.id.page_conn) }
        findViewById<View>(R.id.nav_settings).setOnClickListener { showPage(R.id.page_settings) }
        showPage(page)
    }

    private fun showPage(id: Int) {
        page = id
        pages.forEach { (pageId, view) -> view.visibility = if (pageId == id) View.VISIBLE else View.GONE }
        val items = listOf(
            Triple(R.id.page_home, R.id.nav_home, R.id.nav_home_icon to R.id.nav_home_text),
            Triple(R.id.page_conn, R.id.nav_conn, R.id.nav_conn_icon to R.id.nav_conn_text),
            Triple(R.id.page_settings, R.id.nav_settings, R.id.nav_settings_icon to R.id.nav_settings_text),
        )
        for ((pageId, itemId, parts) in items) {
            val selected = pageId == id
            val color = getColor(if (selected) R.color.accent else R.color.muted)
            findViewById<LinearLayout>(itemId).background =
                if (selected) getDrawable(R.drawable.nav_selected) else null
            findViewById<ImageView>(parts.first).setColorFilter(color)
            findViewById<TextView>(parts.second).setTextColor(if (selected) getColor(R.color.accent_dark) else color)
        }
    }

    private fun copyAddress() {
        val text = wifi.text?.toString()?.takeIf { it.contains(':') } ?: return
        (getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager)
            .setPrimaryClip(ClipData.newPlainText("OpenCam", text))
        Toast.makeText(this, "Dirección copiada: $text", Toast.LENGTH_SHORT).show()
    }

    private fun appVersion(): String = runCatching {
        "v" + packageManager.getPackageInfo(packageName, 0).versionName
    }.getOrDefault("")

    private fun deviceName(): String =
        Settings.Global.getString(contentResolver, "device_name")?.takeIf { it.isNotBlank() } ?: Build.MODEL

    companion object {
        private const val REQ_CAMERA = 1
    }
}
