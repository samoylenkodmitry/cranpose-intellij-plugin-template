package dev.cranpose.intellij

import java.awt.Graphics2D
import java.awt.Rectangle
import java.awt.RenderingHints
import java.awt.geom.AffineTransform
import java.awt.image.BufferedImage
import java.awt.image.DataBufferInt
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.ceil
import kotlin.math.floor

/**
 * The pixels of one surface. Frames arrive on the session's reader thread and
 * are painted on the UI thread, scaled from physical to logical pixels.
 */
class FrameCanvas(scale: Double) {
    private val lock = Any()
    private var image: BufferedImage? = null
    private var imageScale = scale

    /** The scale of the size last sent for this surface; a new frame buffer is drawn at it. */
    @Volatile
    var scale: Double = scale

    /** Copies [frame] in and returns the logical area it changed. */
    fun apply(frame: AppEvent.Frame): Rectangle {
        val scale = synchronized(lock) {
            val target = image?.takeIf { it.width == frame.bufferWidth && it.height == frame.bufferHeight }
                ?: BufferedImage(frame.bufferWidth, frame.bufferHeight, BufferedImage.TYPE_INT_ARGB_PRE).also {
                    image = it
                    imageScale = scale
                }
            val destination = (target.raster.dataBuffer as DataBufferInt).data
            val source = ByteBuffer.wrap(frame.pixels).order(ByteOrder.LITTLE_ENDIAN).asIntBuffer()
            for (row in 0 until frame.height) {
                source.position(row * frame.width)
                source.get(destination, (frame.y + row) * target.width + frame.x, frame.width)
            }
            imageScale
        }
        return Rectangle(
            floor(frame.x / scale).toInt(),
            floor(frame.y / scale).toInt(),
            ceil(frame.width / scale).toInt() + 1,
            ceil(frame.height / scale).toInt() + 1,
        )
    }

    /** Forgets the pixels, for when the process has gone. */
    fun clear() {
        synchronized(lock) { image = null }
    }

    /** Returns an independent copy suitable for exporting on a background thread. */
    fun snapshot(): BufferedImage? = synchronized(lock) {
        val source = image ?: return null
        BufferedImage(source.width, source.height, BufferedImage.TYPE_INT_ARGB).also { copy ->
            copy.createGraphics().let { graphics ->
                graphics.drawImage(source, 0, 0, null)
                graphics.dispose()
            }
        }
    }

    /** Paints the latest frame at the origin; `false` when there is none yet. */
    fun paint(g: Graphics2D): Boolean = synchronized(lock) {
        val shown = image ?: return false
        g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR)
        g.drawImage(shown, AffineTransform.getScaleInstance(1 / imageScale, 1 / imageScale), null)
        true
    }
}
