package dev.cranpose.intellij

import java.awt.Color
import java.awt.Cursor
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.Point
import java.nio.file.Path
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import javax.swing.SwingUtilities

/**
 * A Swing component that shows a Cranpose process and forwards input to it.
 *
 * [command] starts the process; [start] launches it off the UI thread and
 * [close] stops it. The panel is the process's primary surface; the windows
 * the process opens become [SurfaceWindow]s, and its overlays are handed to
 * the IDE layer to place. The component knows nothing about IntelliJ: an IDE
 * layer sets the `on…` callbacks and calls [send] and [setTheme].
 */
class CranposePanel(
    private val command: () -> List<String>,
    private val workingDirectory: Path? = null,
    private val log: (String) -> Unit = {},
) : SurfaceView(CranposeProtocol.PRIMARY_SURFACE, SessionLink()), AutoCloseable {
    /** Called on the UI thread each time a process has connected. */
    var onConnected: () -> Unit = {}

    /** Called on the session's reader thread for every message the process sends. */
    var onAppMessage: (channel: String, payload: String) -> Unit = { _, _ -> }

    /** Called on the UI thread when the process opens an overlay; the IDE layer places it. */
    var onOverlayOpened: (SurfaceOverlay) -> Unit = {}

    /** Called on the UI thread when an overlay closes, after it left its parent. */
    var onOverlayClosed: (SurfaceOverlay) -> Unit = {}

    private val launcher = Executors.newSingleThreadExecutor { runnable ->
        Thread(runnable, "cranpose-panel-launcher").apply { isDaemon = true }
    }
    private val input = SurfaceInput(this)

    /** The process's other surfaces. Their canvases exist from the command on, their Swing parts once built. */
    private val secondary = ConcurrentHashMap<Int, Secondary>()

    private class Secondary(val canvas: FrameCanvas) {
        @Volatile
        var view: SurfaceView? = null
        var input: SurfaceInput? = null
        var window: SurfaceWindow? = null
    }

    /** Delivers one process's events; it goes stale when the panel stops or replaces that process. */
    private inner class Connection : CranposeListener {
        @Volatile
        var live = true

        override fun onFrame(frame: AppEvent.Frame) {
            if (live) showFrame(frame)
        }

        override fun onCursor(surface: Int, name: String) {
            if (live) showCursor(surface, name)
        }

        override fun onMessage(channel: String, payload: String) {
            if (live) onAppMessage(channel, payload)
        }

        override fun onCommand(command: SurfaceCommand) {
            if (live) handle(command)
        }

        override fun onExit(error: Throwable?) {
            if (live) exited(error)
        }
    }

    @Volatile
    private var session: CranposeSession? = null

    private var connection: Connection? = null

    @Volatile
    private var status: String = "Starting Cranpose…"

    @Volatile
    private var starting = false

    private var dark = true

    init {
        isFocusable = true
        focusTraversalKeysEnabled = false
        isOpaque = true
        background = Color(0x2B2D30)
        foreground = Color(0x868A91)
        input.onPress = {
            requestFocusInWindow()
            val connected = session != null
            if (!connected) start()
            connected
        }
    }

    /** Starts the process, replacing any that is running. */
    fun start() {
        if (starting || launcher.isShutdown) return
        starting = true
        launcher.execute {
            stopSession()
            status = "Starting Cranpose…"
            repaint()
            try {
                val listener = Connection()
                val started = CranposeSession.start(command(), workingDirectory, listener, log)
                connection = listener
                session = started
                link.host = started.host
                started.host.theme(dark)
                SwingUtilities.invokeLater {
                    sendSize()
                    sendVisibility()
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
        link.host?.message(channel, payload)
    }

    /** Tells the process whether the IDE uses a dark theme. */
    fun setTheme(dark: Boolean) {
        this.dark = dark
        link.host?.theme(dark)
    }

    override fun close() {
        if (launcher.isShutdown) return
        launcher.execute(::stopSession)
        launcher.shutdown()
    }

    private fun showFrame(frame: AppEvent.Frame) {
        if (frame.surface == surface) {
            repaint(canvas.apply(frame))
        } else {
            secondary[frame.surface]?.let { target ->
                val changed = target.canvas.apply(frame)
                target.view?.repaint(changed)
            }
        }
        link.host?.frameAck(frame.surface, frame.frameId)
    }

    private fun showCursor(surface: Int, name: String) {
        SwingUtilities.invokeLater {
            val target = if (surface == this.surface) this else secondary[surface]?.view
            target?.cursor = Cursor.getPredefinedCursor(cursorType(name))
        }
    }

    private fun handle(command: SurfaceCommand) {
        when (command) {
            is SurfaceCommand.OpenWindow -> {
                secondary.computeIfAbsent(command.surface) { Secondary(FrameCanvas(canvas.scale)) }
                SwingUtilities.invokeLater { openWindow(command.spec) }
            }
            is SurfaceCommand.OpenOverlay -> {
                secondary.computeIfAbsent(command.surface) { Secondary(FrameCanvas(canvas.scale)) }
                SwingUtilities.invokeLater { openOverlay(command.surface, command.anchor) }
            }
            is SurfaceCommand.Close -> secondary.remove(command.surface)?.let { closed ->
                SwingUtilities.invokeLater { dismiss(closed) }
            }
            is SurfaceCommand.BeginMove -> SwingUtilities.invokeLater { secondary[command.surface]?.input?.begin(null) }
            is SurfaceCommand.BeginResize -> SwingUtilities.invokeLater {
                secondary[command.surface]?.input?.begin(command.edge)
            }
        }
    }

    private fun exited(error: Throwable?) {
        closeSurfaces()
        if (error != null) {
            status = "Cranpose stopped: ${error.message}. Click to restart."
            canvas.clear()
            repaint()
        }
    }

    override fun paintComponent(graphics: Graphics) {
        val g = graphics as Graphics2D
        g.color = background
        g.fillRect(0, 0, width, height)
        if (!canvas.paint(g)) {
            g.color = foreground
            g.drawString(status, 12, 24)
        }
    }

    private fun stopSession() {
        connection?.live = false
        connection = null
        link.host = null
        session?.close()
        session = null
        closeSurfaces()
    }

    private fun openWindow(spec: WindowSpec) {
        val target = secondary[spec.surface] ?: return
        target.window?.let {
            it.update(spec)
            return
        }
        val view = SurfaceView(spec.surface, link, target.canvas).apply { isFocusable = true }
        target.input = SurfaceInput(view)
        target.view = view
        target.window = SurfaceWindow(SwingUtilities.getWindowAncestor(this), spec, view, ::hostOrigin)
    }

    private fun openOverlay(surface: Int, anchor: String) {
        val target = secondary[surface] ?: return
        if (target.view != null) return
        val overlay = SurfaceOverlay(anchor, surface, link, target.canvas)
        target.view = overlay
        onOverlayOpened(overlay)
    }

    private fun dismiss(closed: Secondary) {
        closed.window?.close()
        (closed.view as? SurfaceOverlay)?.let { overlay ->
            overlay.removeFromParent()
            onOverlayClosed(overlay)
        }
    }

    private fun closeSurfaces() {
        val closed = secondary.values.toList()
        secondary.clear()
        if (closed.isNotEmpty()) SwingUtilities.invokeLater { closed.forEach(::dismiss) }
    }

    private fun hostOrigin(): Point? = if (isShowing) locationOnScreen else null

    companion object {
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
