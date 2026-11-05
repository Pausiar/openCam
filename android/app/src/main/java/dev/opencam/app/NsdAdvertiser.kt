package dev.opencam.app

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.provider.Settings

/** Anuncia `_opencam._tcp` por mDNS para que el escritorio encuentre el móvil en la Wi-Fi. */
class NsdAdvertiser(private val context: Context, private val port: Int = Opcm.DEFAULT_PORT) {
    private val nsd = context.getSystemService(Context.NSD_SERVICE) as NsdManager
    private var listener: NsdManager.RegistrationListener? = null

    fun start() {
        val deviceName = Settings.Global.getString(context.contentResolver, "device_name")
            ?.takeIf { it.isNotBlank() } ?: Build.MODEL
        val info = NsdServiceInfo().apply {
            serviceName = deviceName
            serviceType = "_opencam._tcp"
            port = this@NsdAdvertiser.port
            setAttribute("platform", "android")
            setAttribute("name", deviceName)
        }
        val l = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(i: NsdServiceInfo) = Unit
            override fun onRegistrationFailed(i: NsdServiceInfo, errorCode: Int) = Unit
            override fun onServiceUnregistered(i: NsdServiceInfo) = Unit
            override fun onUnregistrationFailed(i: NsdServiceInfo, errorCode: Int) = Unit
        }
        listener = l
        runCatching { nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, l) }
    }

    fun stop() {
        listener?.let { runCatching { nsd.unregisterService(it) } }
        listener = null
    }
}
