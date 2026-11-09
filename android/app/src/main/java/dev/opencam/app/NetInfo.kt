package dev.opencam.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import java.net.Inet4Address

/** Datos de red para mostrar al usuario cómo conectar el ordenador. */
object NetInfo {
    /**
     * IPv4 del móvil en la Wi-Fi (o Ethernet), o null si no hay red local.
     * Solo informativa: el escritorio lo encuentra solo por mDNS; la IP sirve si la red bloquea multicast.
     */
    fun localIpv4(context: Context): String? {
        val cm = context.getSystemService(Context.CONNECTIVITY_SERVICE) as? ConnectivityManager ?: return null
        val net = cm.activeNetwork ?: return null
        val caps = cm.getNetworkCapabilities(net) ?: return null
        val local = caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        if (!local) return null
        return cm.getLinkProperties(net)?.linkAddresses
            ?.map { it.address }
            ?.firstOrNull { it is Inet4Address && !it.isLoopbackAddress }
            ?.hostAddress
    }
}
