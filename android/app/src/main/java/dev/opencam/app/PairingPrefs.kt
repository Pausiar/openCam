package dev.opencam.app

import android.content.Context

/**
 * Emparejamientos guardados en SharedPreferences privadas de la app: id del móvil y una
 * clave de 32 bytes por PC. Las copias de seguridad están desactivadas en el manifiesto
 * para que las claves no salgan del teléfono.
 */
class PrefsPairingKeys(context: Context) : PairingKeys {
    private val prefs = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    override fun get(clientId: String): ByteArray? =
        prefs.getString(KEY_PREFIX + clientId, null)?.let { Secure.unhex(it) }?.takeIf { it.size == 32 }

    override fun put(clientId: String, key: ByteArray) {
        prefs.edit().putString(KEY_PREFIX + clientId, Secure.hex(key)).apply()
    }

    override fun clear() {
        val e = prefs.edit()
        prefs.all.keys.filter { it.startsWith(KEY_PREFIX) }.forEach { e.remove(it) }
        e.apply()
    }

    fun phoneId(): String =
        prefs.getString(PHONE_ID, null)?.takeIf { Secure.isId(it) }
            ?: Secure.hex(Secure.randomBytes(16)).also { prefs.edit().putString(PHONE_ID, it).apply() }

    companion object {
        private const val FILE = "opencam_pairing"
        private const val KEY_PREFIX = "pc_"
        private const val PHONE_ID = "phone_id"
    }
}

object PairingPrefs {
    /** Estado de emparejamiento del servicio; publica el código en [ConnectionState]. */
    fun pairing(context: Context): Pairing {
        val keys = PrefsPairingKeys(context.applicationContext)
        return Pairing(keys.phoneId(), keys).also { p ->
            p.onCodeChanged = { code ->
                ConnectionState.pairingCode = code
                ConnectionState.listener?.invoke(ConnectionState.connected)
            }
            // La pantalla ya se pintó al pulsar Iniciar, antes de que el servicio tuviera código:
            // hay que avisarla también del primero, no solo de los cambios.
            ConnectionState.pairingCode = p.code
            ConnectionState.listener?.invoke(ConnectionState.connected)
        }
    }
}
