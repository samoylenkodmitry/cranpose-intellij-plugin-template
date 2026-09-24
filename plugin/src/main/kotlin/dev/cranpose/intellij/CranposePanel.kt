package dev.cranpose.intellij

import java.awt.Color
import java.awt.Cursor
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.RenderingHints
import java.awt.event.ComponentAdapter
import java.awt.event.ComponentEvent
import java.awt.event.HierarchyEvent
import java.awt.event.KeyEvent
import java.awt.event.KeyListener
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import java.awt.event.MouseWheelEvent
import java.awt.geom.AffineTransform
import java.awt.image.BufferedImage
import java.awt.image.DataBufferInt
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Path
import java.util.concurrent.Executors
import javax.swing.JComponent
import javax.swing.SwingUtilities
import kotlin.math.ceil
import kotlin.math.floor

/**
 * A Swing component that shows a Cranpose process and forwards input to it.
 *
 * [command] starts the process; [start] launches it off the UI thread and
 * [close] stops it. The component knows nothing about IntelliJ: an IDE layer
 * sets [onConnected] and [onAppMessage] and calls [send] and [setTheme].
 */
class CranposePanel(
    private val command: () -> List<String>,
    private val workingDirectory: Path? = null,
    private val log: (String) -> Unit = {},
) : JComponent(), CranposeListener, AutoCloseable {
    /** Called on the UI thread each time a process has connected. */
    var onConnected: () -> Unit = {}

    /** Called on the session's reader thread for every message the process sends. */
    var onAppMessage: (channel: String, payload: String) -> Unit = { _, _ -> }

    private val launcher = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "cranpose-panel-launcher").apply { isDaemon = true }
    }
    private val frameLock = Any()
    private var image: BufferedImage? = null
    private var imageScale = 1.0

    @Volatile
    private var session: CranposeSession? = null

    @Volatile
    private var status: String = "Starting Cranpose…"

    @Volatile
    private var starting = false

    @Volatile
    private var sentScale = 1.0

    private var dark = true
    private var pressed = false

    init {
        isFocusable = true
        focusTraversalKeysEnabled = false
        isOpaque = true
        background = Color(0x2B2D30)
        foreground = Color(0x868A91)
        installInput()
        addComponentListener(object : ComponentAdapter() {
            override fun componentResized(event: ComponentEvent) = sendSize()
        })
        addPropertyChangeListener("graphicsConfiguration") { sendSize() }
        addHierarchyListener { event ->
            if (event.changeFlags and HierarchyEvent.SHOWING_CHANGED.toLong() != 0L) {
                session?.host?.visibility(isShowing)
            }
        }
    }

    /** Starts the process, replacing any that is running. */
    fun start() {
        if (starting || launcher.isShutdown) return
        starting = true
        launcher.execute {
            session?.close()
            session = null
            status = "Starting Cranpose…"
            repaint()
            try {
                val started = CranposeSession.start(command(), workingDirectory, this, log)
                session = started
                started.host.theme(dark)
                SwingUtilities.invokeLater {
                    sendSize()
                    started.host.visibility(isShowing)
                    onConnected()
                }
            } catch (error: Exception) {
                status = "Cranpose failed to start: ${error.message}. Click to retry."
                log("cranpose: ${error.stackTraceToString()}")
                repaint()
            } finally {
                starting = false
            }
        }
    }

    /** Sends [payload] on [channel] to the process; dropped while none is connected. */
    fun send(channel: String, payload: String) {
        session?.host?.message(channel, payload)
    }

    /** Tells the process whether the IDE uses a dark theme. */
    fun setTheme(dark: Boolean) {
        this.dark = dark
        session?.host?.theme(dark)
    }

    override fun close() {
        if (launcher.isShutdown) return
        launcher.execute {
            session?.close()
            session = null
        }
        launcher.shutdown()
    }

    override fun onFrame(frame: AppEvent.Frame) {
        synchronized(frameLock) {
            val target = image?.takeIf { it.width == frame.bufferWidth && it.height == frame.bufferHeight }
                ?: BufferedImage(frame.bufferWidth, frame.bufferHeight, BufferedImage.TYPE_INT_ARGB_PRE).also {
                    image = it
                    imageScale = sentScale
                }
            val destination = (target.raster.dataBuffer as DataBufferInt).data
            val source = ByteBuffer.wrap(frame.pixels).order(ByteOrder.LITTLE_ENDIAN).asIntBuffer()
            for (row in 0 until frame.height) {
                source.position(row * frame.width)
                source.get(destination, (frame.y + row) * target.width + frame.x, frame.width)
            }
        }
        session?.host?.frameAck(frame.frameId)
        val scale = imageScale
        repaint(
            floor(frame.x / scale).toInt(),
            floor(frame.y / scale).toInt(),
            ceil(frame.width / scale).toInt() + 1,
            ceil(frame.height / scale).toInt() + 1,
        )
    }

    override fun onCursor(name: String) {
        SwingUtilities.invokeLater { cursor = Cursor.getPredefinedCursor(cursorType(name)) }
    }

    override fun onMessage(channel: String, payload: String) = onAppMessage(channel, payload)

    override fun onExit(error: Throwable?) {
        if (error != null) {
            status = "Cranpose stopped: ${error.message}. Click to restart."
            synchronized(frameLock) { image = null }
            repaint()
        }
    }

    override fun paintComponent(graphics: Graphics) {
        val g = graphics as Graphics2D
        g.color = background
        g.fillRect(0, 0, width, height)
        synchronized(frameLock) {
            val shown = image
            if (shown == null) {
                g.color = foreground
                g.drawString(status, 12, 24)
                return
            }
            g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR)
            g.drawImage(shown, AffineTransform.getScaleInstance(1 / imageScale, 1 / imageScale), null)
        }
    }

    private val requestedScale: Double
        get() = graphicsConfiguration?.defaultTransform?.scaleX ?: 1.0

    private fun sendSize() {
        val host = session?.host ?: return
        if (width <= 0 || height <= 0) return
        val scale = requestedScale
        sentScale = scale
        val refresh = graphicsConfiguration?.device?.displayMode?.refreshRate?.takeIf { it > 0 } ?: 60
        host.resize(ceil(width * scale).toInt(), ceil(height * scale).toInt(), scale.toFloat(), refresh.toFloat())
    }

    private fun installInput() {
        val mouse = object : MouseAdapter() {
            override fun mousePressed(event: MouseEvent) {
                requestFocusInWindow()
                if (session == null) {
                    start()
                    return
                }
                if (SwingUtilities.isLeftMouseButton(event)) {
                    pressed = true
                    session?.host?.pointerDown(event.x.toFloat(), event.y.toFloat())
                }
            }

            override fun mouseReleased(event: MouseEvent) {
                if (SwingUtilities.isLeftMouseButton(event) && pressed) {
                    pressed = false
                    session?.host?.pointerUp(event.x.toFloat(), event.y.toFloat())
                }
            }

            override fun mouseMoved(event: MouseEvent) {
                session?.host?.pointerMove(event.x.toFloat(), event.y.toFloat())
            }

            override fun mouseDragged(event: MouseEvent) = mouseMoved(event)

            override fun mouseExited(event: MouseEvent) {
                if (!pressed) session?.host?.pointerLeave()
            }

            override fun mouseWheelMoved(event: MouseWheelEvent) {
                val delta = -event.preciseWheelRotation.toFloat() * LINE_PIXELS
                val horizontal = event.isShiftDown
                session?.host?.scroll(
                    event.x.toFloat(),
                    event.y.toFloat(),
                    if (horizontal) delta else 0f,
                    if (horizontal) 0f else delta,
                    KeyCodes.modifiers(event.modifiersEx),
                )
            }
        }
        addMouseListener(mouse)
        addMouseMotionListener(mouse)
        addMouseWheelListener(mouse)
        addKeyListener(object : KeyListener {
            override fun keyPressed(event: KeyEvent) = key(event, down = true)

            override fun keyReleased(event: KeyEvent) = key(event, down = false)

            override fun keyTyped(event: KeyEvent) {
                if (KeyCodes.insertsText(event.keyChar, event.modifiersEx)) {
                    session?.host?.text(event.keyChar.toString())
                    event.consume()
                }
            }
        })
    }

    private fun key(event: KeyEvent, down: Boolean) {
        val code = KeyCodes.domCode(event.keyCode, event.keyLocation) ?: return
        session?.host?.key(down, KeyCodes.modifiers(event.modifiersEx), code)
        event.consume()
    }

    companion object {
        private const val LINE_PIXELS = 40f

        /** The AWT cursor closest to a CSS cursor name. */
        fun cursorType(name: String): Int = when (name) {
            "pointer", "grab", "grabbing" -> Cursor.HAND_CURSOR
            "text", "vertical-text" -> Cursor.TEXT_CURSOR
            "crosshair", "cell" -> Cursor.CROSSHAIR_CURSOR
            "move", "all-scroll" -> Cursor.MOVE_CURSOR
            "wait", "progress" -> Cursor.WAIT_CURSOR
            "ew-resize", "col-resize", "e-resize" -> Cursor.E_RESIZE_CURSOR
            "w-resize" -> Cursor.W_RESIZE_CURSOR
            "ns-resize", "row-resize", "n-resize" -> Cursor.N_RESIZE_CURSOR
            "s-resize" -> Cursor.S_RESIZE_CURSOR
            "nwse-resize", "nw-resize" -> Cursor.NW_RESIZE_CURSOR
            "se-resize" -> Cursor.SE_RESIZE_CURSOR
            "nesw-resize", "ne-resize" -> Cursor.NE_RESIZE_CURSOR
            "sw-resize" -> Cursor.SW_RESIZE_CURSOR
            else -> Cursor.DEFAULT_CURSOR
        }
    }
}
