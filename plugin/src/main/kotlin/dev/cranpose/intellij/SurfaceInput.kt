package dev.cranpose.intellij

import java.awt.Dimension
import java.awt.Point
import java.awt.Rectangle
import java.awt.event.KeyEvent
import java.awt.event.KeyListener
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import java.awt.event.MouseWheelEvent
import javax.swing.SwingUtilities

/**
 * Forwards a [SurfaceView]'s mouse, wheel and key input to the process, and
 * carries out the window moves and resizes the process starts from a press
 * with [SurfaceCommand.BeginMove] and [SurfaceCommand.BeginResize].
 */
class SurfaceInput(private val view: SurfaceView) {
    /** Runs on every press before it is forwarded; returning `false` swallows the press. */
    var onPress: (MouseEvent) -> Boolean = { view.requestFocusInWindow(); true }

    private var pressed = false
    private var pressedAt: Point? = null
    private var gesture: Gesture? = null

    private class Gesture(val edge: ResizeEdge?, val start: Rectangle, val from: Point) {
        var travelled = false
    }

    init {
        val mouse = object : MouseAdapter() {
            override fun mousePressed(event: MouseEvent) {
                if (!onPress(event) || !SwingUtilities.isLeftMouseButton(event)) return
                pressed = true
                pressedAt = event.locationOnScreen
                host()?.pointerDown(view.surface, logical(event.x), logical(event.y))
            }

            override fun mouseReleased(event: MouseEvent) {
                if (!SwingUtilities.isLeftMouseButton(event) || !pressed) return
                pressed = false
                val ended = gesture
                gesture = null
                // After a real move or resize the press must not end in a click.
                val released = if (ended?.travelled == true) Point(-1, -1) else event.point
                host()?.pointerUp(view.surface, logical(released.x), logical(released.y))
            }

            override fun mouseMoved(event: MouseEvent) {
                host()?.pointerMove(view.surface, logical(event.x), logical(event.y))
            }

            override fun mouseDragged(event: MouseEvent) {
                val running = gesture ?: return mouseMoved(event)
                follow(running, event.locationOnScreen)
            }

            override fun mouseExited(event: MouseEvent) {
                if (!pressed) host()?.pointerLeave(view.surface)
            }

            override fun mouseWheelMoved(event: MouseWheelEvent) {
                val delta = -event.preciseWheelRotation.toFloat() * LINE_PIXELS
                val horizontal = event.isShiftDown
                host()?.scroll(
                    view.surface,
                    logical(event.x),
                    logical(event.y),
                    if (horizontal) delta else 0f,
                    if (horizontal) 0f else delta,
                    KeyCodes.modifiers(event.modifiersEx),
                )
            }
        }
        view.addMouseListener(mouse)
        view.addMouseMotionListener(mouse)
        view.addMouseWheelListener(mouse)
        view.addKeyListener(object : KeyListener {
            override fun keyPressed(event: KeyEvent) = key(event, down = true)

            override fun keyReleased(event: KeyEvent) = key(event, down = false)

            override fun keyTyped(event: KeyEvent) {
                if (KeyCodes.insertsText(event.keyChar, event.modifiersEx)) {
                    host()?.text(view.surface, event.keyChar.toString())
                    event.consume()
                }
            }
        })
    }

    /**
     * Moves ([edge] `null`) or resizes the view's window with the pointer until
     * the button that pressed it is released. Ignored once it is released.
     */
    fun begin(edge: ResizeEdge?) {
        val window = SwingUtilities.getWindowAncestor(view) ?: return
        val from = pressedAt ?: return
        if (pressed) gesture = Gesture(edge, window.bounds, from)
    }

    private fun follow(running: Gesture, pointer: Point) {
        val window = SwingUtilities.getWindowAncestor(view) ?: return
        val dx = pointer.x - running.from.x
        val dy = pointer.y - running.from.y
        if (!running.travelled && dx * dx + dy * dy < CLICK_SLOP * CLICK_SLOP) return
        running.travelled = true
        window.bounds = running.edge?.let { resized(running.start, it, dx, dy, window.minimumSize) }
            ?: Rectangle(running.start.x + dx, running.start.y + dy, running.start.width, running.start.height)
    }

    private fun key(event: KeyEvent, down: Boolean) {
        val code = KeyCodes.domCode(event.keyCode, event.keyLocation) ?: return
        host()?.key(view.surface, down, KeyCodes.modifiers(event.modifiersEx), code)
        event.consume()
    }

    private fun host(): HostWriter? = view.link.host

    private fun logical(value: Int): Float = (value / view.contentScale).toFloat()

    companion object {
        private const val LINE_PIXELS = 40f
        private const val CLICK_SLOP = 4
        private const val SMALLEST = 48

        /**
         * [start] resized from [edge] by a pointer that travelled ([dx], [dy]),
         * no smaller than [minimum] or 48 pixels, the opposite edges staying put.
         */
        fun resized(start: Rectangle, edge: ResizeEdge, dx: Int, dy: Int, minimum: Dimension?): Rectangle {
            val minWidth = maxOf(minimum?.width ?: 0, SMALLEST)
            val minHeight = maxOf(minimum?.height ?: 0, SMALLEST)
            val west = edge in setOf(ResizeEdge.WEST, ResizeEdge.NORTH_WEST, ResizeEdge.SOUTH_WEST)
            val east = edge in setOf(ResizeEdge.EAST, ResizeEdge.NORTH_EAST, ResizeEdge.SOUTH_EAST)
            val north = edge in setOf(ResizeEdge.NORTH, ResizeEdge.NORTH_EAST, ResizeEdge.NORTH_WEST)
            val south = edge in setOf(ResizeEdge.SOUTH, ResizeEdge.SOUTH_EAST, ResizeEdge.SOUTH_WEST)
            val width = maxOf(minWidth, start.width + if (east) dx else if (west) -dx else 0)
            val height = maxOf(minHeight, start.height + if (south) dy else if (north) -dy else 0)
            val x = if (west) start.x + start.width - width else start.x
            val y = if (north) start.y + start.height - height else start.y
            return Rectangle(x, y, width, height)
        }
    }
}
