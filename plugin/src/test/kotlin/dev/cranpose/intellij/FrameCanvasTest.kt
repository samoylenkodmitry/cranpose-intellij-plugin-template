package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.awt.Rectangle
import java.awt.image.BufferedImage

class FrameCanvasTest {
    private fun frame(x: Int, y: Int, width: Int, height: Int, bgra: Int) = AppEvent.Frame(
        surface = 0,
        frameId = 1,
        bufferWidth = 8,
        bufferHeight = 8,
        x = x,
        y = y,
        width = width,
        height = height,
        pixels = ByteArray(width * height * 4) { (bgra ushr (it % 4 * 8)).toByte() },
    )

    private fun painted(canvas: FrameCanvas): BufferedImage = BufferedImage(4, 4, BufferedImage.TYPE_INT_ARGB).also { image ->
        val g = image.createGraphics()
        assertTrue(canvas.paint(g))
        g.dispose()
    }

    @Test
    fun framesPaintAtLogicalSizeAndReportWhatChanged() {
        val canvas = FrameCanvas(2.0)
        assertFalse("nothing to paint before the first frame", canvas.paint(BufferedImage(1, 1, BufferedImage.TYPE_INT_ARGB).createGraphics()))
        assertEquals(Rectangle(0, 0, 5, 5), canvas.apply(frame(0, 0, 8, 8, 0xFF0000FF.toInt())))
        assertEquals(Rectangle(2, 1, 2, 2), canvas.apply(frame(4, 2, 2, 2, 0xFF00FF00.toInt())))
        val image = painted(canvas)
        assertEquals("BGRA bytes read as ARGB", 0xFF0000FF.toInt(), image.getRGB(0, 0))
        assertEquals(0xFF00FF00.toInt(), image.getRGB(2, 1))
    }

    @Test
    fun transparentPixelsStayTransparent() {
        val canvas = FrameCanvas(2.0)
        canvas.apply(frame(0, 0, 8, 8, 0))
        assertEquals(0, painted(canvas).getRGB(1, 1) ushr 24)
        canvas.clear()
        assertFalse(canvas.paint(BufferedImage(1, 1, BufferedImage.TYPE_INT_ARGB).createGraphics()))
    }

    @Test
    fun exportedPixelsAreIndependentFromSubsequentFrames() {
        val canvas = FrameCanvas(2.0)
        canvas.apply(frame(0, 0, 8, 8, 0xFF00FF00.toInt()))
        val exported = requireNotNull(canvas.snapshot())
        canvas.apply(frame(0, 0, 8, 8, 0xFFFF0000.toInt()))
        assertEquals(8, exported.width)
        assertEquals(0xFF00FF00.toInt(), exported.getRGB(0, 0))
        assertEquals(0xFFFF0000.toInt(), requireNotNull(canvas.snapshot()).getRGB(0, 0))
    }
}
