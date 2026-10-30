package dev.opencam.app

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : Activity() {

    private lateinit var status: TextView
    private lateinit var toggle: Button

    private val running get() = ConnectionState.running

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        status = findViewById(R.id.status)
        toggle = findViewById(R.id.toggle)

        toggle.setOnClickListener {
            if (ConnectionState.running) stopStream() else requestPermissionsAndStart()
        }
    }

    override fun onResume() {
        super.onResume()
        ConnectionState.listener = { runOnUiThread { refresh() } }
        refresh()
    }

    override fun onPause() {
        ConnectionState.listener = null
        super.onPause()
    }

    private fun requestPermissionsAndStart() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            startStream()
        } else {
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.CAMERA), REQ_CAMERA)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQ_CAMERA && grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
            startStream()
        } else if (requestCode == REQ_CAMERA) {
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
