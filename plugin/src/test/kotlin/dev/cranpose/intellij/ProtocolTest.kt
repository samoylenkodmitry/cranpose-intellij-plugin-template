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
        val bytes = byteArrayOf(0x0B, 0, 0, 0, 0x01, 0x01, 0, 0, 0, 0x02, 0, 0, 0, 'a'.code.toByte(), 'b'.code.toByte())
        assertEquals(AppEvent.Hello(1, "ab"), AppReader(ByteArrayInputStream(bytes)).next())
    }

    @Test
    fun aFrameCarriesItsRectangleAndPixels() {
        val pixels = (16 until 24).toList().toIntArray()
        val bytes = framed(0x02, *le(9, 3, 2, 1, 1, 2, 1), *pixels)
        val frame = AppReader(ByteArrayInputStream(bytes)).next() as AppEvent.Frame
        assertEquals(listOf(9, 3, 2, 1, 1, 2, 1), listOf(frame.frameId, frame.bufferWidth, frame.bufferHeight, frame.x, frame.y, frame.width, frame.height))
        assertArrayEquals(ByteArray(8) { (16 + it).toByte() }, frame.pixels)
    }

    @Test
    fun cursorAndMessageDecodeAndUnknownKindsAreSkipped() {
        val stream = framed(0x7F, 1, 2) +
            framed(0x03, *le(4), 't'.code, 'e'.code, 'x'.code, 't'.code) +
            framed(0x04, *le(1), 'c'.code, *le(2), '{'.code, '}'.code)
        val reader = AppReader(ByteArrayInputStream(stream))
        assertEquals(AppEvent.Cursor("text"), reader.next())
        assertEquals(AppEvent.Message("c", "{}"), reader.next())
        assertNull(reader.next())
    }

    @Test(expected = EOFException::class)
    fun aTruncatedMessageFails() {
        AppReader(ByteArrayInputStream(byteArrayOf(4, 0, 0, 0, 0x03, 1))).next()
    }

    @Test
    fun hostEventsEncodeToTheBytesTheRustSideDecodes() {
        assertArrayEquals(framed(0x01, *le(640, 480, 2.0f.toRawBits(), 120.0f.toRawBits())), encode { resize(640, 480, 2f, 120f) })
        assertArrayEquals(framed(0x02, *le(3.0f.toRawBits(), 4.0f.toRawBits())), encode { pointerMove(3f, 4f) })
        assertArrayEquals(framed(0x03, *le(10.5f.toRawBits(), 20.0f.toRawBits())), encode { pointerDown(10.5f, 20f) })
        assertArrayEquals(framed(0x04, *le(3.0f.toRawBits(), 4.0f.toRawBits())), encode { pointerUp(3f, 4f) })
        assertArrayEquals(framed(0x05), encode { pointerLeave() })
        assertArrayEquals(
            framed(0x06, *le(1.0f.toRawBits(), 2.0f.toRawBits(), 0f.toRawBits(), (-40.0f).toRawBits()), 0b0101),
            encode { scroll(1f, 2f, 0f, -40f, 0b0101) },
        )
        assertArrayEquals(framed(0x07, 1, 0b1010, *le(4), 'K'.code, 'e'.code, 'y'.code, 'A'.code), encode { key(true, 0b1010, "KeyA") })
        assertArrayEquals(framed(0x08, *le(2), 0xC3, 0xA9), encode { text("é") })
        assertArrayEquals(framed(0x09, 0), encode { theme(false) })
        assertArrayEquals(framed(0x0A, *le(1), 'c'.code, *le(1), 'p'.code), encode { message("c", "p") })
        assertArrayEquals(framed(0x0B, *le(7)), encode { frameAck(7) })
        assertArrayEquals(framed(0x0C), encode { close() })
        assertArrayEquals(framed(0x0D, 0), encode { visibility(false) })
    }

    @Test
    fun modifierBitsMatchTheRustSide() {
        assertTrue(
            CranposeProtocol.MODIFIER_SHIFT == 1 && CranposeProtocol.MODIFIER_CTRL == 2 &&
                CranposeProtocol.MODIFIER_ALT == 4 && CranposeProtocol.MODIFIER_META == 8,
        )
    }
}
