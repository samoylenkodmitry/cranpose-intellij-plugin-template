package dev.cranpose.intellij

import java.awt.AlphaComposite
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.event.ComponentAdapter
import java.awt.event.ComponentEvent
import java.awt.event.HierarchyEvent
import javax.swing.JComponent
import kotlin.math.ceil

/** The connection every surface of one panel writes to; `null` while no process is connected. */
class SessionLink {
    @Volatile
    var host: HostWriter? = null
}

/**
 * A component that shows one surface of a Cranpose process: it paints the
 * surface's [canvas] and reports its size, scale and visibility to the process.
 */
open class SurfaceView(
    val surface: Int,
    val link: SessionLink,
    val canvas: FrameCanvas = FrameCanvas(1.0),
) : JComponent() {
    init {
        addComponentListener(object : ComponentAdapter() {
            override fun componentResized(event: ComponentEvent) = sendSize()
        })
        addPropertyChangeListener("graphicsConfiguration") { sendSize() }
        addHierarchyListener { event ->
            if (event.changeFlags and HierarchyEvent.SHOWING_CHANGED.toLong() != 0L) sendVisibility()
        }
    }

    /**
     * Whether this view is all of a transparent window, so each paint sets
     * every pixel's alpha instead of drawing over what was there.
     */
    var fillsTransparentWindow = false

    /** Physical pixels per logical pixel on the screen showing this component. */
    val screenScale: Double
        get() = graphicsConfiguration?.defaultTransform?.scaleX ?: canvas.scale

    /** Sends the component's size in physical pixels; nothing while it has none. */
    fun sendSize() {
        val host = link.host ?: return
        if (width <= 0 || height <= 0) return
        val scale = screenScale
        canvas.scale = scale
        val refresh = graphicsConfiguration?.device?.displayMode?.refreshRate?.takeIf { it > 0 } ?: 60
        host.resize(surface, ceil(width * scale).toInt(), ceil(height * scale).toInt(), scale.toFloat(), refresh.toFloat())
    }

    fun sendVisibility() {
        link.host?.visibility(surface, isShowing)
    }

    override fun paintComponent(graphics: Graphics) {
        val g = graphics as Graphics2D
        when {
            isOpaque -> {
                g.color = background
                g.fillRect(0, 0, width, height)
            }
            fillsTransparentWindow -> {
                g.composite = AlphaComposite.Clear
                g.fillRect(0, 0, width, height)
                g.composite = AlphaComposite.SrcOver
            }
        }
        canvas.paint(g)
    }
}
