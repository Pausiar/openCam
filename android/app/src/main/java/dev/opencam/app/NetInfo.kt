package dev.opencam.app

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import java.net.Inet4Address
import java.net.NetworkInterface

/** Datos de red para mostrar al usuario cómo conectar el ordenador. */
object NetInfo {
    data class Address(val ip: String, val kind: String)

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

    /**
     * Todas las IPv4 por las que el PC puede llegar al móvil: Wi-Fi, punto de acceso del propio
     * móvil, Ethernet o anclaje USB. Las de datos móviles se omiten (el PC no las alcanza).
     */
    fun addresses(context: Context): List<Address> {
        val found = runCatching {
            NetworkInterface.getNetworkInterfaces().toList()
                .filter { it.isUp && !it.isLoopback }
                .flatMap { nif ->
                    nif.inetAddresses.toList()
                        .filterIsInstance<Inet4Address>()
                        .filter { !it.isLoopbackAddress && !it.isLinkLocalAddress }
                        .map { Address(it.hostAddress ?: "", kindOf(nif.name)) }
                }
                .filter { it.ip.isNotEmpty() && it.kind.isNotEmpty() }
        }.getOrDefault(emptyList())
        // La red activa (si es Wi-Fi/Ethernet) siempre aparece, aunque su interfaz tenga otro nombre.
        val active = localIpv4(context)?.takeIf { ip -> found.none { it.ip == ip } }?.let { Address(it, "Wi-Fi") }
        return (listOfNotNull(active) + found).distinctBy { it.ip }.sortedBy { ORDER.indexOf(it.kind) }
    }

    /** Tipo de red por nombre de interfaz; "" para las que el PC no puede alcanzar (datos móviles, VPN). */
    fun kindOf(name: String): String = when {
        name.startsWith("wlan") -> "Wi-Fi"
        name.startsWith("ap") || name.startsWith("swlan") -> "Punto de acceso"
        name.startsWith("eth") -> "Ethernet"
        name.startsWith("rndis") || name.startsWith("usb") || name.startsWith("ncm") -> "USB (anclaje)"
        name.startsWith("rmnet") || name.startsWith("ccmni") || name.startsWith("tun") -> ""
        else -> "Red"
    }

    private val ORDER = listOf("Wi-Fi", "Punto de acceso", "Ethernet", "USB (anclaje)", "Red")
}
