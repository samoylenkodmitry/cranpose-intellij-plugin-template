package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.awt.Point
import java.awt.Rectangle
import java.awt.image.BufferedImage
import java.awt.image.DataBufferInt
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import javax.imageio.ImageIO
import kotlin.math.abs

/**
 * Runs the real UI binary the way the tool window does, without the IDE: the
 * test plays the host, reads the frames it streams back, and clicks the
 * buttons it finds in them.
 */
class EmbeddedUiEndToEndTest {
    private val binary: Path? = System.getProperty("cranpose.ui.binary")?.let(Path::of)?.takeIf(Files::isExecutable)
    private val output: Path? = System.getProperty("cranpose.test.output")?.let(Path::of)

    private class Exit(val error: Throwable?)

    private class Recorder : CranposeListener {
        val events = LinkedBlockingQueue<Any>()

        override fun onFrame(frame: AppEvent.Frame) = events.put(frame)

        override fun onMessage(channel: String, payload: String) = events.put(AppEvent.Message(channel, payload))

        override fun onCommand(command: SurfaceCommand) = events.put(command)

        override fun onExit(error: Throwable?) = events.put(Exit(error))
    }

    private class Canvas {
        var image = BufferedImage(1, 1, BufferedImage.TYPE_INT_ARGB_PRE)

        fun apply(frame: AppEvent.Frame) {
            if (image.width != frame.bufferWidth || image.height != frame.bufferHeight) {
                image = BufferedImage(frame.bufferWidth, frame.bufferHeight, BufferedImage.TYPE_INT_ARGB_PRE)
            }
            val destination = (image.raster.dataBuffer as DataBufferInt).data
            val source = ByteBuffer.wrap(frame.pixels).order(ByteOrder.LITTLE_ENDIAN).asIntBuffer()
            for (y in 0 until frame.height) {
                source.position(y * frame.width)
                source.get(destination, (frame.y + y) * image.width + frame.x, frame.width)
            }
        }

        fun pixel(x: Int, y: Int): Int = image.getRGB(x, y)

        fun alpha(x: Int, y: Int): Int = pixel(x, y) ushr 24

        fun find(color: Int, rows: IntProgression): Point? {
            for (y in rows) for (x in 0 until image.width) if (close(pixel(x, y), color)) return Point(x, y)
            return null
        }

        fun rowHas(color: Int, y: Int): Boolean = (0 until image.width).any { close(pixel(it, y), color) }

        /** The runs of row [y] whose pixels are [inside]. */
        fun runs(y: Int, inside: (Int) -> Boolean): List<IntRange> {
            val found = ArrayList<IntRange>()
            var start = -1
            for (x in 0..image.width) {
                val hit = x < image.width && inside(pixel(x, y))
                if (hit && start < 0) start = x
                if (!hit && start >= 0) {
                    found += start until x
                    start = -1
                }
            }
            return found
        }

        /** The top-left of the next [color] button below the one at [above]. */
        fun buttonBelow(color: Int, above: Point): Point? {
            var y = above.y
            while (y < image.height && rowHas(color, y)) y++
            return find(color, y until image.height)
        }

        fun opaquePixels(): Int {
            var count = 0
            for (y in 0 until image.height step 2) for (x in 0 until image.width step 2) if (alpha(x, y) > 16) count++
            return count
        }
    }

    private class Driver(private val session: CranposeSession, private val recorder: Recorder) {
        val canvases = HashMap<Int, Canvas>()
        val commands = ArrayList<SurfaceCommand>()
        val canvas: Canvas get() = canvasOf(CranposeProtocol.PRIMARY_SURFACE)

        fun canvasOf(surface: Int): Canvas = canvases.getOrPut(surface) { Canvas() }

        fun next(timeoutMillis: Long): Any? {
            val event = recorder.events.poll(timeoutMillis, TimeUnit.MILLISECONDS)
            if (event is AppEvent.Frame) {
                canvasOf(event.surface).apply(event)
                session.host.frameAck(event.surface, event.frameId)
            }
            if (event is SurfaceCommand) commands += event
            if (event is Exit) fail("the UI process exited: ${event.error}")
            return event
        }

        fun awaitCommand(timeoutMillis: Long = 10_000, accept: (SurfaceCommand) -> Boolean): SurfaceCommand {
            commands.firstOrNull(accept)?.let { return it }
            val deadline = System.currentTimeMillis() + timeoutMillis
            while (System.currentTimeMillis() < deadline) {
                val event = next(deadline - System.currentTimeMillis())
                if (event is SurfaceCommand && accept(event)) return event
            }
            fail("no matching command within $timeoutMillis ms")
            error("unreachable")
        }

        fun awaitFrame(timeoutMillis: Long = 20_000, accept: (AppEvent.Frame) -> Boolean): AppEvent.Frame {
            val deadline = System.currentTimeMillis() + timeoutMillis
            while (System.currentTimeMillis() < deadline) {
                val event = next(deadline - System.currentTimeMillis())
                if (event is AppEvent.Frame && accept(event)) return event
            }
            fail("no matching frame within $timeoutMillis ms")
            error("unreachable")
        }

        fun awaitCanvas(surface: Int = CranposeProtocol.PRIMARY_SURFACE, timeoutMillis: Long = 20_000, accept: (Canvas) -> Boolean) {
            awaitFrame(timeoutMillis) { it.surface == surface && accept(canvasOf(surface)) }
        }

        fun awaitQuiet(quietMillis: Long, timeoutMillis: Long = 10_000): Boolean {
            val deadline = System.currentTimeMillis() + timeoutMillis
            while (System.currentTimeMillis() < deadline) {
                if (next(quietMillis) == null) return true
            }
            return false
        }

        /** Takes whatever arrives for [millis]. */
        fun pump(millis: Long) {
            val deadline = System.currentTimeMillis() + millis
            while (System.currentTimeMillis() < deadline) next(maxOf(1, deadline - System.currentTimeMillis()))
        }

        fun awaitMessage(timeoutMillis: Long = 10_000): AppEvent.Message {
            val deadline = System.currentTimeMillis() + timeoutMillis
            while (System.currentTimeMillis() < deadline) {
                val event = next(deadline - System.currentTimeMillis())
                if (event is AppEvent.Message) return event
            }
            fail("no message within $timeoutMillis ms")
            error("unreachable")
        }

        fun click(physical: Point) {
            val x = physical.x / SCALE
            val y = physical.y / SCALE
            val primary = CranposeProtocol.PRIMARY_SURFACE
            session.host.pointerMove(primary, x, y)
            session.host.pointerDown(primary, x, y)
            session.host.pointerUp(primary, x, y)
        }
    }

    @Test
    fun theToolWindowRendersAnimatesAndTalksToTheIde() {
        assumeTrue("set cranpose.ui.binary to the built UI to run this test", binary != null)
        val recorder = Recorder()
        CranposeSession.start(listOf(binary.toString()), null, recorder, log = { println("[ui] $it") }).use { session ->
            val driver = Driver(session, recorder)
            session.host.theme(true)
            session.host.message(IdeBridge.THEME_CHANNEL, THEME)
            session.host.resize(CranposeProtocol.PRIMARY_SURFACE, WIDTH, HEIGHT, SCALE, 60f)

            val first = driver.awaitFrame { it.surface == CranposeProtocol.PRIMARY_SURFACE && it.bufferWidth == WIDTH && it.bufferHeight == HEIGHT }
            assertEquals(Rectangle(0, 0, WIDTH, HEIGHT), Rectangle(first.x, first.y, first.width, first.height))
            driver.awaitCanvas { close(it.pixel(4, HEIGHT - 4), BACKGROUND) }
            save(driver.canvas, "1-themed.png")

            val animated = driver.awaitFrame { it.surface == CranposeProtocol.PRIMARY_SURFACE && (it.width < WIDTH || it.height < HEIGHT) }
            val card = Rectangle(animated.x, animated.y, animated.width, animated.height)
            assertTrue("the shader animates only its own card, got $card", card.height in 200..420 && card.y > 0)

            val pause = driver.canvas.find(ACCENT, (card.y + card.height) until HEIGHT)
            assertNotNull("the Pause button is drawn below the card", pause)
            driver.click(Point(pause!!.x + 8, pause.y + 8))
            assertTrue("pausing the shader stops the frames", driver.awaitQuiet(quietMillis = 800))
            save(driver.canvas, "2-paused.png")

            editorEffects(session, driver)
            val orbButton = driver.canvas.buttonBelow(ACCENT, pause)
            floatingOrb(session, driver, orbButton)
            effectStyles(driver, orbButton!!)

            session.host.message(IdeBridge.EDITOR_CHANNEL, FlatJson.encode("path" to "/tmp/Hello.rs", "name" to "Hello.rs"))
            val editor = driver.awaitFrame { it.surface == CranposeProtocol.PRIMARY_SURFACE && it.y > card.y + card.height }
            assertTrue("only the editor card repaints for a new file", editor.height < HEIGHT / 3)
            driver.awaitQuiet(quietMillis = 300)
            save(driver.canvas, "3-editor.png")

            val send = driver.canvas.find(ACCENT, (HEIGHT - 1) downTo (card.y + card.height))
            assertNotNull("the Send notification button is drawn", send)
            driver.click(Point(send!!.x + 8, send.y - 8))
            assertEquals(
                AppEvent.Message(IdeBridge.NOTIFY_CHANNEL, """{"content":"Hello from Cranpose!","title":"Cranpose"}"""),
                driver.awaitMessage(),
            )
        }
    }

    /** The overlay the tool window lays over the editor draws sparks where the caret goes, then clears. */
    private fun editorEffects(session: CranposeSession, driver: Driver) {
        val overlay = driver.awaitCommand { it is SurfaceCommand.OpenOverlay } as SurfaceCommand.OpenOverlay
        assertEquals(EditorOverlay.ANCHOR, overlay.anchor)
        session.host.resize(overlay.surface, OVERLAY_WIDTH, OVERLAY_HEIGHT, SCALE, 60f)
        driver.awaitCanvas(overlay.surface) { it.image.width == OVERLAY_WIDTH }
        assertEquals("the overlay is transparent while nothing happens", 0, driver.canvasOf(overlay.surface).opaquePixels())

        session.host.message(
            EditorOverlay.CARET_CHANNEL,
            EditorOverlay.caretPayload("type", Point(120, 100), Point(110, 100), Rectangle(0, 0, 400, 300), 16),
        )
        driver.awaitCanvas(overlay.surface) { it.opaquePixels() > 20 }
        driver.pump(200)
        save(driver.canvasOf(overlay.surface), "4-sparks.png")
        assertTrue("the sparks burn out and the frames stop", driver.awaitQuiet(quietMillis = 600))
        assertEquals("the overlay is clear again", 0, driver.canvasOf(overlay.surface).opaquePixels())
    }

    /** The orb button opens a transparent window shaped by its shader; pressing it again closes it. */
    private fun floatingOrb(session: CranposeSession, driver: Driver, button: Point?) {
        assertNotNull("the Floating orb button is drawn below the Pause row", button)
        driver.click(Point(button!!.x + 8, button.y + 8))
        val open = driver.awaitCommand { it is SurfaceCommand.OpenWindow } as SurfaceCommand.OpenWindow
        val spec = open.spec
        assertTrue("the orb is a borderless transparent window", spec.has(CranposeProtocol.WINDOW_TRANSPARENT) && !spec.has(CranposeProtocol.WINDOW_DECORATED))
        assertTrue("the orb opens beside the tool window", spec.relativeToHost && (spec.position?.first ?: 0f) < 0f)
        val side = (spec.width * SCALE).toInt()
        driver.awaitCanvas(open.surface) { it.image.width == side && it.alpha(side / 2, side / 2) == 255 }
        val orb = driver.canvasOf(open.surface)
        assertEquals("the orb's corners are see-through", 0, orb.alpha(2, 2))

        val resized = side + 40
        session.host.visibility(open.surface, true)
        session.host.resize(open.surface, resized, resized, SCALE, 60f)
        driver.awaitCanvas(open.surface) { it.image.width == resized && it.alpha(resized / 2, resized / 2) == 255 }
        assertEquals("the resized orb's corners stay see-through", 0, orb.alpha(2, 2))
        save(orb, "5-orb.png")

        driver.click(Point(button.x + 8, button.y + 8))
        driver.awaitCommand { it == SurfaceCommand.Close(open.surface) }
        driver.awaitQuiet(quietMillis = 300)
    }

    /** Each style's four effects, clicked into the playground, saved as a picture per style. */
    private fun effectStyles(driver: Driver, orbButton: Point) {
        val canvas = driver.canvas
        val firstChip = canvas.buttonBelow(ACCENT, orbButton)
        assertNotNull("the style chips are drawn below the orb button", firstChip)
        val chipY = firstChip!!.y + 3
        val chips = canvas.runs(chipY) { close(it, ACCENT) || close(it, BACKGROUND) }.filter { it.last - it.first > 30 }
        assertEquals("one chip per style", STYLES.size, chips.size)
        val top = (chipY + 40 until HEIGHT).first { y -> (0 until WIDTH).count { close(canvas.pixel(it, y), BACKGROUND) } > WIDTH * 8 / 10 }
        val area = Rectangle(48, top, WIDTH - 96, PLAYGROUND_HEIGHT)
        val clicks = listOf(0.15 to 0.55, 0.4 to 0.6, 0.85 to 0.3, 0.62 to 0.75)
        STYLES.zip(chips).forEach { (style, chip) ->
            driver.click(Point((chip.first + chip.last) / 2, chipY + 4))
            driver.awaitQuiet(quietMillis = 200)
            clicks.forEach { (x, y) -> driver.click(Point(area.x + (area.width * x).toInt(), area.y + (area.height * y).toInt())) }
            driver.pump(280)
            val drawn = (area.y until area.y + area.height step 2).sumOf { y ->
                (area.x until area.x + area.width step 2).count { x -> !close(canvas.pixel(x, y), BACKGROUND) }
            }
            assertTrue("$style draws in the playground", drawn > 100)
            save(canvas.image.getSubimage(area.x, area.y, area.width, area.height), "6-$style.png")
            assertTrue("$style's effects burn out", driver.awaitQuiet(quietMillis = 700))
        }
    }

    private fun save(canvas: Canvas, name: String) = save(canvas.image, name)

    private fun save(image: BufferedImage, name: String) {
        val directory = output ?: return
        Files.createDirectories(directory)
        ImageIO.write(image, "png", directory.resolve(name).toFile())
    }

    private companion object {
        const val WIDTH = 840
        const val HEIGHT = 1800
        const val PLAYGROUND_HEIGHT = 220
        val STYLES = listOf("sparks", "water", "ice", "cosmic", "party")
        const val OVERLAY_WIDTH = 800
        const val OVERLAY_HEIGHT = 600
        const val SCALE = 2f
        const val BACKGROUND = 0xFF2B2D30.toInt()
        const val ACCENT = 0xFF3574F0.toInt()
        val THEME = FlatJson.encode(
            "dark" to true,
            "background" to "#2b2d30",
            "surface" to "#393b40",
            "text" to "#dfe1e5",
            "muted" to "#868a91",
            "accent" to "#3574f0",
        )

        fun close(argb: Int, expected: Int): Boolean = (0..16 step 8).all { shift ->
            abs((argb shr shift and 0xFF) - (expected shr shift and 0xFF)) <= 2
        }
    }
}
