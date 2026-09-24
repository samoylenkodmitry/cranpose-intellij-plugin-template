package dev.cranpose.intellij

import java.io.EOFException
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * The wire format between the IDE and a Cranpose process, mirrored from
 * `crates/cranpose/src/embed_protocol.rs`.
 *
 * Every message is a little-endian `u32` body length followed by the body: a
 * one-byte kind and its fields. Strings are a `u32` byte length and UTF-8
 * bytes; pixels are BGRA rows, premultiplied. Everything the process draws is
 * a surface: [PRIMARY_SURFACE] is the tool window, and the process opens more
 * for its windows and overlays.
 */
object CranposeProtocol {
    const val VERSION = 2

    /** The surface the host embeds the process in. */
    const val PRIMARY_SURFACE = 0

    const val HOST_RESIZE: Byte = 0x01
    const val HOST_POINTER_MOVE: Byte = 0x02
    const val HOST_POINTER_DOWN: Byte = 0x03
    const val HOST_POINTER_UP: Byte = 0x04
    const val HOST_POINTER_LEAVE: Byte = 0x05
    const val HOST_SCROLL: Byte = 0x06
    const val HOST_KEY: Byte = 0x07
    const val HOST_TEXT: Byte = 0x08
    const val HOST_THEME: Byte = 0x09
    const val HOST_MESSAGE: Byte = 0x0A
    const val HOST_FRAME_ACK: Byte = 0x0B
    const val HOST_CLOSE: Byte = 0x0C
    const val HOST_VISIBILITY: Byte = 0x0D
    const val HOST_MOVED: Byte = 0x0E
    const val HOST_CLOSE_REQUESTED: Byte = 0x0F

    const val APP_HELLO: Byte = 0x01
    const val APP_FRAME: Byte = 0x02
    const val APP_CURSOR: Byte = 0x03
    const val APP_MESSAGE: Byte = 0x04
    const val APP_OPEN_WINDOW: Byte = 0x05
    const val APP_OPEN_OVERLAY: Byte = 0x06
    const val APP_CLOSE_SURFACE: Byte = 0x07
    const val APP_BEGIN_MOVE: Byte = 0x08
    const val APP_BEGIN_RESIZE: Byte = 0x09

    const val MODIFIER_SHIFT = 1
    const val MODIFIER_CTRL = 1 shl 1
    const val MODIFIER_ALT = 1 shl 2
    const val MODIFIER_META = 1 shl 3

    const val WINDOW_DECORATED = 1
    const val WINDOW_TRANSPARENT = 1 shl 1
    const val WINDOW_RESIZABLE = 1 shl 2
    const val WINDOW_ALWAYS_ON_TOP = 1 shl 3
    const val WINDOW_SHADOW = 1 shl 4
    const val WINDOW_TAKES_FOCUS = 1 shl 5
}

/** The edge or corner a window is resized from; the order is the wire code. */
enum class ResizeEdge { EAST, NORTH, NORTH_EAST, NORTH_WEST, SOUTH, SOUTH_EAST, SOUTH_WEST, WEST }

/**
 * A window the process asks for, in logical pixels. [position] is `null` when
 * the process leaves it to the host, and relative to the tool window when
 * [relativeToHost] holds.
 */
data class WindowSpec(
    val surface: Int,
    val title: String,
    val position: Pair<Float, Float>?,
    val relativeToHost: Boolean,
    val width: Float,
    val height: Float,
    val flags: Int,
) {
    fun has(flag: Int): Boolean = flags and flag != 0
}

/** A message the Cranpose process sent. */
sealed interface AppEvent {
    data class Hello(val version: Int, val token: String) : AppEvent

    /**
     * The pixels of [width]×[height] at ([x], [y]) in [surface]'s frame buffer
     * of [bufferWidth]×[bufferHeight]; [pixels] holds the rectangle's BGRA rows.
     */
    class Frame(
        val surface: Int,
        val frameId: Int,
        val bufferWidth: Int,
        val bufferHeight: Int,
        val x: Int,
        val y: Int,
        val width: Int,
        val height: Int,
        val pixels: ByteArray,
    ) : AppEvent

    data class Cursor(val surface: Int, val name: String) : AppEvent

    data class Message(val channel: String, val payload: String) : AppEvent
}

/** What the process asks the host to do with one of its surfaces. */
sealed interface SurfaceCommand : AppEvent {
    val surface: Int

    /** Open a window for [spec], or update the one [spec] names. */
    data class OpenWindow(val spec: WindowSpec) : SurfaceCommand {
        override val surface: Int get() = spec.surface
    }

    /** Lay a transparent surface over the part of the IDE [anchor] names. */
    data class OpenOverlay(override val surface: Int, val anchor: String) : SurfaceCommand

    data class Close(override val surface: Int) : SurfaceCommand

    /** Move the window with the pointer until the button is released. */
    data class BeginMove(override val surface: Int) : SurfaceCommand

    /** Resize the window from [edge] with the pointer until the button is released. */
    data class BeginResize(override val surface: Int, val edge: ResizeEdge) : SurfaceCommand
}

/** Encodes host messages. Safe to call from any thread. */
class HostWriter(private val out: OutputStream) {
    @Synchronized
    fun resize(surface: Int, width: Int, height: Int, scale: Float, refreshHz: Float) =
        send(CranposeProtocol.HOST_RESIZE, surface) { u32(width); u32(height); f32(scale); f32(refreshHz) }

    @Synchronized
    fun pointerMove(surface: Int, x: Float, y: Float) = send(CranposeProtocol.HOST_POINTER_MOVE, surface) { f32(x); f32(y) }

    @Synchronized
    fun pointerDown(surface: Int, x: Float, y: Float) = send(CranposeProtocol.HOST_POINTER_DOWN, surface) { f32(x); f32(y) }

    @Synchronized
    fun pointerUp(surface: Int, x: Float, y: Float) = send(CranposeProtocol.HOST_POINTER_UP, surface) { f32(x); f32(y) }

    @Synchronized
    fun pointerLeave(surface: Int) = send(CranposeProtocol.HOST_POINTER_LEAVE, surface) {}

    @Synchronized
    fun scroll(surface: Int, x: Float, y: Float, deltaX: Float, deltaY: Float, modifiers: Int) =
        send(CranposeProtocol.HOST_SCROLL, surface) { f32(x); f32(y); f32(deltaX); f32(deltaY); u8(modifiers) }

    @Synchronized
    fun key(surface: Int, down: Boolean, modifiers: Int, code: String) = send(CranposeProtocol.HOST_KEY, surface) {
        u8(if (down) 1 else 0); u8(modifiers); string(code)
    }

    @Synchronized
    fun text(surface: Int, text: String) = send(CranposeProtocol.HOST_TEXT, surface) { string(text) }

    @Synchronized
    fun theme(dark: Boolean) = send(CranposeProtocol.HOST_THEME) { u8(if (dark) 1 else 0) }

    @Synchronized
    fun message(channel: String, payload: String) = send(CranposeProtocol.HOST_MESSAGE) {
        string(channel); string(payload)
    }

    @Synchronized
    fun frameAck(surface: Int, frameId: Int) = send(CranposeProtocol.HOST_FRAME_ACK, surface) { u32(frameId) }

    @Synchronized
    fun close() = send(CranposeProtocol.HOST_CLOSE) {}

    @Synchronized
    fun visibility(surface: Int, visible: Boolean) =
        send(CranposeProtocol.HOST_VISIBILITY, surface) { u8(if (visible) 1 else 0) }

    /** [surface]'s window now sits at ([x], [y]), in the space its [WindowSpec] asked for. */
    @Synchronized
    fun moved(surface: Int, x: Float, y: Float) = send(CranposeProtocol.HOST_MOVED, surface) { f32(x); f32(y) }

    @Synchronized
    fun closeRequested(surface: Int) = send(CranposeProtocol.HOST_CLOSE_REQUESTED, surface) {}

    private fun send(kind: Byte, surface: Int, fields: Body.() -> Unit) = send(kind) { u32(surface); fields() }

    private fun send(kind: Byte, fields: Body.() -> Unit) {
        val body = Body().apply { u8(kind.toInt()) }.apply(fields)
        val header = ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(body.size).array()
        out.write(header)
        out.write(body.bytes, 0, body.size)
        out.flush()
    }

    private class Body {
        var bytes = ByteArray(32)
        var size = 0

        fun u8(value: Int) {
            reserve(1)
            bytes[size++] = value.toByte()
        }

        fun u32(value: Int) {
            reserve(4)
            ByteBuffer.wrap(bytes, size, 4).order(ByteOrder.LITTLE_ENDIAN).putInt(value)
            size += 4
        }

        fun f32(value: Float) = u32(java.lang.Float.floatToRawIntBits(value))

        fun string(value: String) {
            val utf8 = value.toByteArray(Charsets.UTF_8)
            u32(utf8.size)
            reserve(utf8.size)
            System.arraycopy(utf8, 0, bytes, size, utf8.size)
            size += utf8.size
        }

        private fun reserve(extra: Int) {
            if (size + extra > bytes.size) bytes = bytes.copyOf(maxOf(bytes.size * 2, size + extra))
        }
    }
}

/** Decodes the messages a Cranpose process sends. Not thread-safe; one reader thread owns it. */
class AppReader(private val input: InputStream) {
    private var body = ByteArray(64 * 1024)

    /** The next event, or `null` when the process closed the stream cleanly. */
    fun next(): AppEvent? {
        while (true) {
            val header = ByteArray(4)
            if (!readFully(header, allowEof = true)) return null
            val length = ByteBuffer.wrap(header).order(ByteOrder.LITTLE_ENDIAN).int
            if (length <= 0 || length > MAX_BODY) throw IOException("app message of $length bytes is out of range")
            if (body.size < length) body = ByteArray(length)
            readFully(body, length = length)
            decode(length)?.let { return it }
        }
    }

    private fun decode(length: Int): AppEvent? {
        val fields = ByteBuffer.wrap(body, 0, length).order(ByteOrder.LITTLE_ENDIAN)
        return when (fields.get()) {
            CranposeProtocol.APP_HELLO -> AppEvent.Hello(fields.int, fields.string())
            CranposeProtocol.APP_FRAME -> frame(fields)
            CranposeProtocol.APP_CURSOR -> AppEvent.Cursor(fields.int, fields.string())
            CranposeProtocol.APP_MESSAGE -> AppEvent.Message(fields.string(), fields.string())
            CranposeProtocol.APP_OPEN_WINDOW -> SurfaceCommand.OpenWindow(windowSpec(fields))
            CranposeProtocol.APP_OPEN_OVERLAY -> SurfaceCommand.OpenOverlay(fields.int, fields.string())
            CranposeProtocol.APP_CLOSE_SURFACE -> SurfaceCommand.Close(fields.int)
            CranposeProtocol.APP_BEGIN_MOVE -> SurfaceCommand.BeginMove(fields.int)
            CranposeProtocol.APP_BEGIN_RESIZE -> {
                val surface = fields.int
                ResizeEdge.entries.getOrNull(fields.u8())?.let { SurfaceCommand.BeginResize(surface, it) }
            }
            else -> null
        }
    }

    private fun frame(fields: ByteBuffer): AppEvent.Frame {
        val surface = fields.int
        val frameId = fields.int
        val bufferWidth = fields.int
        val bufferHeight = fields.int
        val x = fields.int
        val y = fields.int
        val width = fields.int
        val height = fields.int
        val bytes = width.toLong() * height * 4
        if (bytes != fields.remaining().toLong()) throw IOException("frame of ${width}x$height carries ${fields.remaining()} bytes")
        val pixels = body.copyOfRange(fields.position(), fields.position() + bytes.toInt())
        return AppEvent.Frame(surface, frameId, bufferWidth, bufferHeight, x, y, width, height, pixels)
    }

    private fun windowSpec(fields: ByteBuffer): WindowSpec {
        val surface = fields.int
        val title = fields.string()
        val x = fields.float
        val y = fields.float
        val width = fields.float
        val height = fields.float
        val relativeToHost = fields.u8() != 0
        val flags = fields.u8()
        val position = if (x.isNaN() || y.isNaN()) null else x to y
        return WindowSpec(surface, title, position, relativeToHost, width, height, flags)
    }

    private fun readFully(buffer: ByteArray, length: Int = buffer.size, allowEof: Boolean = false): Boolean {
        var filled = 0
        while (filled < length) {
            val read = input.read(buffer, filled, length - filled)
            if (read < 0) {
                if (allowEof && filled == 0) return false
                throw EOFException("stream ended inside a message")
            }
            filled += read
        }
        return true
    }

    private fun ByteBuffer.u8(): Int = get().toInt() and 0xFF

    private fun ByteBuffer.string(): String {
        val length = int
        if (length < 0 || length > remaining()) throw IOException("string of $length bytes overruns its message")
        val text = String(array(), arrayOffset() + position(), length, Charsets.UTF_8)
        position(position() + length)
        return text
    }

    private companion object {
        const val MAX_BODY = 512 * 1024 * 1024
    }
}
