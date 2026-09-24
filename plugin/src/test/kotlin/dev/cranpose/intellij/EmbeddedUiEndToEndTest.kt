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

        override fun onExit(error: Throwable?) = events.put(Exit(error))
    }

    private class Canvas {
        var image = BufferedImage(1, 1, BufferedImage.TYPE_INT_ARGB)

        fun apply(frame: AppEvent.Frame) {
            if (image.width != frame.bufferWidth || image.height != frame.bufferHeight) {
                image = BufferedImage(frame.bufferWidth, frame.bufferHeight, BufferedImage.TYPE_INT_ARGB)
            }
            val row = IntArray(frame.width)
            val source = ByteBuffer.wrap(frame.pixels).order(ByteOrder.LITTLE_ENDIAN).asIntBuffer()
            for (y in 0 until frame.height) {
                source.position(y * frame.width)
                source.get(row)
                image.setRGB(frame.x, frame.y + y, frame.width, 1, row, 0, frame.width)
            }
        }

        fun pixel(x: Int, y: Int): Int = image.getRGB(x, y)

        fun find(color: Int, rows: IntProgression): Point? {
            for (y in rows) for (x in 0 until image.width) if (close(pixel(x, y), color)) return Point(x, y)
            return null
        }
    }

    private class Driver(private val session: CranposeSession, private val recorder: Recorder, val canvas: Canvas) {
        fun next(timeoutMillis: Long): Any? {
            val event = recorder.events.poll(timeoutMillis, TimeUnit.MILLISECONDS)
            if (event is AppEvent.Frame) {
                canvas.apply(event)
                session.host.frameAck(event.frameId)
            }
            if (event is Exit) fail("the UI process exited: ${event.error}")
            return event
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

        fun awaitCanvas(timeoutMillis: Long = 20_000, accept: (Canvas) -> Boolean) {
            awaitFrame(timeoutMillis) { accept(canvas) }
        }

        fun awaitQuiet(quietMillis: Long, timeoutMillis: Long = 10_000): Boolean {
            val deadline = System.currentTimeMillis() + timeoutMillis
            while (System.currentTimeMillis() < deadline) {
                if (next(quietMillis) == null) return true
            }
            return false
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
            session.host.pointerMove(x, y)
            session.host.pointerDown(x, y)
            session.host.pointerUp(x, y)
        }
    }

    @Test
    fun theToolWindowRendersAnimatesAndTalksToTheIde() {
        assumeTrue("set cranpose.ui.binary to the built UI to run this test", binary != null)
        val recorder = Recorder()
        CranposeSession.start(listOf(binary.toString()), null, recorder, log = { println("[ui] $it") }).use { session ->
            val driver = Driver(session, recorder, Canvas())
            session.host.theme(true)
            session.host.message(IdeBridge.THEME_CHANNEL, THEME)
            session.host.resize(WIDTH, HEIGHT, SCALE, 60f)

            val first = driver.awaitFrame { it.bufferWidth == WIDTH && it.bufferHeight == HEIGHT }
            assertEquals(Rectangle(0, 0, WIDTH, HEIGHT), Rectangle(first.x, first.y, first.width, first.height))
            driver.awaitCanvas { close(it.pixel(4, HEIGHT - 4), BACKGROUND) }
            save(driver.canvas, "1-themed.png")

            val animated = driver.awaitFrame { it.width < WIDTH || it.height < HEIGHT }
            val card = Rectangle(animated.x, animated.y, animated.width, animated.height)
            assertTrue("the shader animates only its own card, got $card", card.height in 200..420 && card.y > 0)

            val pause = driver.canvas.find(ACCENT, (card.y + card.height) until HEIGHT)
            assertNotNull("the Pause button is drawn below the card", pause)
            driver.click(Point(pause!!.x + 8, pause.y + 8))
            assertTrue("pausing the shader stops the frames", driver.awaitQuiet(quietMillis = 800))
            save(driver.canvas, "2-paused.png")

            session.host.message(IdeBridge.EDITOR_CHANNEL, FlatJson.encode("path" to "/tmp/Hello.rs", "name" to "Hello.rs"))
            val editor = driver.awaitFrame { it.y > card.y + card.height }
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

    private fun save(canvas: Canvas, name: String) {
        val directory = output ?: return
        Files.createDirectories(directory)
        ImageIO.write(canvas.image, "png", directory.resolve(name).toFile())
    }

    private companion object {
        const val WIDTH = 840
        const val HEIGHT = 1400
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
