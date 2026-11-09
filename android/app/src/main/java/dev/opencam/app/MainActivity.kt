package dev.opencam.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var toggle: Button
    private lateinit var advanced: AdvancedControls

    private val running get() = ConnectionState.running

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        status = findViewById(R.id.status)
        toggle = findViewById(R.id.toggle)
        advanced = AdvancedControls(findViewById(R.id.advanced))

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
            !running -> "Parado. Conecta el PC con adb forward tcp:4747 tcp:4747"
            ConnectionState.connected -> "Conectado al ordenador"
            else -> "Esperando al ordenador en el puerto ${Opcm.DEFAULT_PORT}"
        }
    }

    companion object {
        private const val REQ_CAMERA = 1
    }
}
