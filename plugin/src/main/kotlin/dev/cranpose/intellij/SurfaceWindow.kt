package dev.cranpose.intellij

import java.awt.Color
import java.awt.Dimension
import java.awt.GraphicsDevice
import java.awt.Point
import java.awt.Window
import java.awt.event.ComponentAdapter
import java.awt.event.ComponentEvent
import java.awt.event.WindowAdapter
import java.awt.event.WindowEvent
import javax.swing.JDialog
import javax.swing.JWindow
import javax.swing.RootPaneContainer
import javax.swing.WindowConstants
import kotlin.math.roundToInt

/**
 * A window the process opened with `Modifier::window`, owned by the IDE frame.
 * A borderless window can be transparent, shaped by what the process draws.
 * [hostOrigin] is the tool window's position on screen, for windows placed
 * relative to it.
 */
class SurfaceWindow(
    private val owner: Window?,
    private var spec: WindowSpec,
    private val view: SurfaceView,
    private val hostOrigin: () -> Point?,
) {
    private var window: Window = create(spec)

    init {
        place(spec, previous = null)
        window.isVisible = true
    }

    /** Applies a changed [next] spec, rebuilding the window when its kind changed. */
    fun update(next: WindowSpec) {
        val previous = spec
        val rebuild = kindFlags(next) != kindFlags(previous)
        spec = next
        if (rebuild) {
            val bounds = window.bounds
            window.dispose()
            window = create(next)
            window.bounds = bounds
            window.isVisible = true
        }
        (window as? JDialog)?.title = next.title
        window.isAlwaysOnTop = next.has(CranposeProtocol.WINDOW_ALWAYS_ON_TOP)
        window.focusableWindowState = next.has(CranposeProtocol.WINDOW_TAKES_FOCUS)
        place(next, previous)
    }

    fun close() {
        window.dispose()
    }

    private fun create(spec: WindowSpec): Window {
        val window = if (spec.has(CranposeProtocol.WINDOW_DECORATED)) {
            JDialog(owner, spec.title).apply {
                isResizable = spec.has(CranposeProtocol.WINDOW_RESIZABLE)
                defaultCloseOperation = WindowConstants.DO_NOTHING_ON_CLOSE
                addWindowListener(object : WindowAdapter() {
                    override fun windowClosing(event: WindowEvent) {
                        view.link.host?.closeRequested(view.surface)
                    }
                })
            }
        } else {
            JWindow(owner)
        }
        val transparent = spec.has(CranposeProtocol.WINDOW_TRANSPARENT) && !spec.has(CranposeProtocol.WINDOW_DECORATED) &&
            window.graphicsConfiguration.device.isWindowTranslucencySupported(GraphicsDevice.WindowTranslucency.PERPIXEL_TRANSLUCENT)
        view.isOpaque = !transparent
        view.fillsTransparentWindow = transparent
        (window as RootPaneContainer).apply {
            contentPane = view
            // Swing's back buffer may have no alpha channel (it has none in the IDE), which paints
            // a transparent window black; the view paints straight onto the window instead.
            rootPane.isDoubleBuffered = !transparent
            rootPane.putClientProperty("Window.shadow", spec.has(CranposeProtocol.WINDOW_SHADOW))
        }
        if (transparent) window.background = Color(0, 0, 0, 0)
        window.isAlwaysOnTop = spec.has(CranposeProtocol.WINDOW_ALWAYS_ON_TOP)
        window.focusableWindowState = spec.has(CranposeProtocol.WINDOW_TAKES_FOCUS)
        window.addComponentListener(object : ComponentAdapter() {
            override fun componentMoved(event: ComponentEvent) = reportPosition()
        })
        return window
    }

    /** Sizes and positions the window as [spec] asks, leaving what [previous] already asked for. */
    private fun place(spec: WindowSpec, previous: WindowSpec?) {
        if (previous == null || previous.width != spec.width || previous.height != spec.height) {
            view.preferredSize = Dimension(spec.width.roundToInt(), spec.height.roundToInt())
            window.pack()
        }
        val origin = if (spec.relativeToHost) hostOrigin() ?: Point() else Point()
        val position = spec.position
        when {
            position != null -> window.setLocation(origin.x + position.first.roundToInt(), origin.y + position.second.roundToInt())
            !window.isVisible -> window.setLocationRelativeTo(owner)
        }
    }

    private fun reportPosition() {
        val location = window.location
        val origin = if (spec.relativeToHost) hostOrigin() ?: Point() else Point()
        view.link.host?.moved(view.surface, (location.x - origin.x).toFloat(), (location.y - origin.y).toFloat())
    }

    private companion object {
        /** The flags a live window cannot change. */
        fun kindFlags(spec: WindowSpec): Int =
            spec.flags and (CranposeProtocol.WINDOW_DECORATED or CranposeProtocol.WINDOW_TRANSPARENT or CranposeProtocol.WINDOW_SHADOW)
    }
}
