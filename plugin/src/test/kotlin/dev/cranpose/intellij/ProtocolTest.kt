package dev.cranpose.intellij

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.EOFException
import java.nio.ByteBuffer
import java.nio.ByteOrder

class ProtocolTest {
    private fun framed(vararg body: Int): ByteArray {
        val bytes = ByteBuffer.allocate(4 + body.size).order(ByteOrder.LITTLE_ENDIAN).putInt(body.size)
        body.forEach { bytes.put(it.toByte()) }
        return bytes.array()
    }

    private fun le(vararg values: Int): IntArray = values.flatMap { value ->
        listOf(value and 0xFF, value shr 8 and 0xFF, value shr 16 and 0xFF, value ushr 24)
    }.toIntArray()

    private fun encode(write: HostWriter.() -> Unit): ByteArray =
        ByteArrayOutputStream().also { HostWriter(it).write() }.toByteArray()

    @Test
    fun helloDecodesTheBytesTheRustSideWrites() {
        val bytes = byteArrayOf(0x0B, 0, 0, 0, 0x01, 0x02, 0, 0, 0, 0x02, 0, 0, 0, 'a'.code.toByte(), 'b'.code.toByte())
        assertEquals(AppEvent.Hello(CranposeProtocol.VERSION, "ab"), AppReader(ByteArrayInputStream(bytes)).next())
    }

    @Test
    fun aFrameCarriesItsRectangleAndPixels() {
        val pixels = (16 until 24).toList().toIntArray()
        val bytes = framed(0x02, *le(5, 9, 3, 2, 1, 1, 2, 1), *pixels)
        val frame = AppReader(ByteArrayInputStream(bytes)).next() as AppEvent.Frame
        assertEquals(
            listOf(5, 9, 3, 2, 1, 1, 2, 1),
            listOf(frame.surface, frame.frameId, frame.bufferWidth, frame.bufferHeight, frame.x, frame.y, frame.width, frame.height),
        )
        assertArrayEquals(ByteArray(8) { (16 + it).toByte() }, frame.pixels)
    }

    @Test
    fun cursorAndMessageDecodeAndUnknownKindsAreSkipped() {
        val stream = framed(0x7F, 1, 2) +
            framed(0x03, *le(2, 4), 't'.code, 'e'.code, 'x'.code, 't'.code) +
            framed(0x04, *le(1), 'c'.code, *le(2), '{'.code, '}'.code)
        val reader = AppReader(ByteArrayInputStream(stream))
        assertEquals(AppEvent.Cursor(2, "text"), reader.next())
        assertEquals(AppEvent.Message("c", "{}"), reader.next())
        assertNull(reader.next())
    }

    @Test
    fun surfaceCommandsDecode() {
        val stream = framed(
            0x05, *le(3, 2), 'O'.code, 'r'.code,
            *le((-174.0f).toRawBits(), 48.0f.toRawBits(), 150.0f.toRawBits(), 150.0f.toRawBits()), 1, 0b10_0010,
        ) +
            framed(0x05, *le(4, 0, Float.NaN.toRawBits(), Float.NaN.toRawBits(), 320.0f.toRawBits(), 200.0f.toRawBits()), 0, 1) +
            framed(0x06, *le(5, 6), 'e'.code, 'd'.code, 'i'.code, 't'.code, 'o'.code, 'r'.code) +
            framed(0x07, *le(3)) +
            framed(0x08, *le(3)) +
            framed(0x09, *le(3), 5) +
            framed(0x09, *le(3), 99)
        val reader = AppReader(ByteArrayInputStream(stream))
        val orb = WindowSpec(3, "Or", -174f to 48f, true, 150f, 150f, 0b10_0010)
        assertEquals(SurfaceCommand.OpenWindow(orb), reader.next())
        assertTrue(orb.has(CranposeProtocol.WINDOW_TRANSPARENT) && orb.has(CranposeProtocol.WINDOW_TAKES_FOCUS))
        assertEquals(SurfaceCommand.OpenWindow(WindowSpec(4, "", null, false, 320f, 200f, 1)), reader.next())
        assertEquals(SurfaceCommand.OpenOverlay(5, "editor"), reader.next())
        assertEquals(SurfaceCommand.Close(3), reader.next())
        assertEquals(SurfaceCommand.BeginMove(3), reader.next())
        assertEquals(SurfaceCommand.BeginResize(3, ResizeEdge.SOUTH_EAST), reader.next())
        assertNull("an unknown resize edge is skipped", reader.next())
    }

    @Test(expected = EOFException::class)
    fun aTruncatedMessageFails() {
        AppReader(ByteArrayInputStream(byteArrayOf(4, 0, 0, 0, 0x03, 1))).next()
    }

    @Test
    fun hostEventsEncodeToTheBytesTheRustSideDecodes() {
        assertArrayEquals(framed(0x01, *le(0, 640, 480, 2.0f.toRawBits(), 120.0f.toRawBits())), encode { resize(0, 640, 480, 2f, 120f) })
        assertArrayEquals(framed(0x02, *le(1, 3.0f.toRawBits(), 4.0f.toRawBits())), encode { pointerMove(1, 3f, 4f) })
        assertArrayEquals(framed(0x03, *le(0, 10.5f.toRawBits(), 20.0f.toRawBits())), encode { pointerDown(0, 10.5f, 20f) })
        assertArrayEquals(framed(0x04, *le(2, 3.0f.toRawBits(), 4.0f.toRawBits())), encode { pointerUp(2, 3f, 4f) })
        assertArrayEquals(framed(0x05, *le(3)), encode { pointerLeave(3) })
        assertArrayEquals(
            framed(0x06, *le(0, 1.0f.toRawBits(), 2.0f.toRawBits(), 0f.toRawBits(), (-40.0f).toRawBits()), 0b0101),
            encode { scroll(0, 1f, 2f, 0f, -40f, 0b0101) },
        )
        assertArrayEquals(framed(0x07, *le(0), 1, 0b1010, *le(4), 'K'.code, 'e'.code, 'y'.code, 'A'.code), encode { key(0, true, 0b1010, "KeyA") })
        assertArrayEquals(framed(0x08, *le(0, 2), 0xC3, 0xA9), encode { text(0, "é") })
        assertArrayEquals(framed(0x09, 0), encode { theme(false) })
        assertArrayEquals(framed(0x0A, *le(1), 'c'.code, *le(1), 'p'.code), encode { message("c", "p") })
        assertArrayEquals(framed(0x0B, *le(4, 7)), encode { frameAck(4, 7) })
        assertArrayEquals(framed(0x0C), encode { close() })
        assertArrayEquals(framed(0x0D, *le(1), 0), encode { visibility(1, false) })
        assertArrayEquals(framed(0x0E, *le(3, (-12.5f).toRawBits(), 40.0f.toRawBits())), encode { moved(3, -12.5f, 40f) })
        assertArrayEquals(framed(0x0F, *le(3)), encode { closeRequested(3) })
    }

    @Test
    fun modifierBitsMatchTheRustSide() {
        assertTrue(
            CranposeProtocol.MODIFIER_SHIFT == 1 && CranposeProtocol.MODIFIER_CTRL == 2 &&
                CranposeProtocol.MODIFIER_ALT == 4 && CranposeProtocol.MODIFIER_META == 8,
        )
    }
}
