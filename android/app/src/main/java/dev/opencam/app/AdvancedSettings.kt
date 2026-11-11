package dev.opencam.app

import android.view.View
import android.widget.CompoundButton
import android.widget.SeekBar
import android.widget.TextView

/**
 * Ajustes avanzados compartidos entre la pantalla y StreamService.
 * El escritorio también los cambia por CONFIG; lo último aplicado se publica aquí.
 */
object AdvancedSettings {
    /** Lo elegido en el móvil; el servicio arranca con esto y lo actualiza al aplicar cambios. */
    @Volatile var current: StreamConfig = StreamConfig()
        private set

    /** La UI escucha los cambios (incluidos los que llegan del escritorio). */
    @Volatile var uiListener: ((StreamConfig) -> Unit)? = null

    /** El servicio, si está activo, aplica los cambios de la UI en su hilo de cámara. */
    @Volatile var serviceHandler: (((StreamConfig) -> StreamConfig) -> Unit)? = null

    fun publish(cfg: StreamConfig) {
        current = cfg
        uiListener?.invoke(cfg)
    }

    /** Cambio pedido desde la pantalla del móvil. */
    fun change(f: (StreamConfig) -> StreamConfig) {
        val handler = serviceHandler
        if (handler != null) handler(f) else publish(f(current))
    }
}

/**
 * Controles de la sección "Ajustes avanzados" (layout advanced_settings.xml).
 * `onRender` recibe cada estado pintado, para que el resto de la pantalla lo refleje.
 */
class AdvancedControls(root: View, private val onRender: (StreamConfig) -> Unit = {}) {
    private val info: TextView = root.findViewById(R.id.adv_info)
    private val front: CompoundButton = root.findViewById(R.id.adv_front)
    private val stab: CompoundButton = root.findViewById(R.id.adv_stab)
    private val zoomLabel: TextView = root.findViewById(R.id.adv_zoom_label)
    private val zoom: SeekBar = root.findViewById(R.id.adv_zoom)
    private val autofocus: CompoundButton = root.findViewById(R.id.adv_af)
    private val focusLabel: TextView = root.findViewById(R.id.adv_focus_label)
    private val focus: SeekBar = root.findViewById(R.id.adv_focus)

    /** Evita que pintar el estado dispare los listeners como si fuera el usuario. */
    private var rendering = false

    init {
        zoom.max = ZOOM_STEPS
        focus.max = FOCUS_STEPS
        front.setOnCheckedChangeListener { _, on -> user { it.copy(facing = if (on) "front" else "back") } }
        stab.setOnCheckedChangeListener { _, on -> user { it.copy(stabilization = on) } }
        autofocus.setOnCheckedChangeListener { _, on -> user { it.copy(focus = if (on) "auto" else "manual") } }
        zoom.setOnSeekBarChangeListener(seek { p -> user { it.copy(zoom = 1f + p / 10f) } })
        focus.setOnSeekBarChangeListener(seek { p -> user { it.copy(focusDistance = p / FOCUS_STEPS.toFloat()) } })
    }

    fun onResume(post: (Runnable) -> Unit) {
        AdvancedSettings.uiListener = { cfg -> post(Runnable { render(cfg) }) }
        render(AdvancedSettings.current)
    }

    fun onPause() {
        AdvancedSettings.uiListener = null
    }

    private fun user(f: (StreamConfig) -> StreamConfig) {
        if (!rendering) AdvancedSettings.change(f)
    }

    private fun render(cfg: StreamConfig) {
        rendering = true
        info.text = "${cfg.codec.uppercase()} · ${cfg.width}x${cfg.height} a ${cfg.fps} fps · ${cfg.bitrateKbps} kbps"
        front.isChecked = cfg.facing == "front"
        stab.isChecked = cfg.stabilization
        zoom.progress = ((cfg.zoom - 1f) * 10f).toInt().coerceIn(0, ZOOM_STEPS)
        zoomLabel.text = "Zoom: %.1fx".format(cfg.zoom)
        val manual = cfg.focus == "manual"
        autofocus.isChecked = !manual
        focus.isEnabled = manual
        focus.progress = (cfg.focusDistance * FOCUS_STEPS).toInt()
        focusLabel.text = if (manual) "Enfoque manual (infinito ↔ cerca)" else "Enfoque manual (desactiva el autoenfoque)"
        rendering = false
        onRender(cfg)
    }

    private fun seek(onUser: (Int) -> Unit) = object : SeekBar.OnSeekBarChangeListener {
        override fun onProgressChanged(bar: SeekBar, progress: Int, fromUser: Boolean) {
            if (fromUser) onUser(progress)
        }
        override fun onStartTrackingTouch(bar: SeekBar) = Unit
        override fun onStopTrackingTouch(bar: SeekBar) = Unit
    }

    companion object {
        /** Zoom 1.0x .. 8.0x en pasos de 0.1; el servicio lo limita al máximo real. */
        private const val ZOOM_STEPS = 70
        private const val FOCUS_STEPS = 100
    }
}
