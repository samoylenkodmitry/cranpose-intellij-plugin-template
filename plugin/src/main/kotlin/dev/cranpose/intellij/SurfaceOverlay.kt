package dev.cranpose.intellij

/**
 * A transparent surface the process draws over the part of the IDE [anchor]
 * names, such as `editor`. It takes no input: clicks, scrolling and keys reach
 * whatever lies beneath. The IDE layer decides where it goes and how big it is.
 */
class SurfaceOverlay(val anchor: String, surface: Int, link: SessionLink, canvas: FrameCanvas) :
    SurfaceView(surface, link, canvas) {
    init {
        isOpaque = false
        isFocusable = false
    }

    override fun contains(x: Int, y: Int): Boolean = false

    /** Takes the overlay out of wherever the IDE layer put it. */
    fun removeFromParent() {
        val container = parent ?: return
        val area = bounds
        container.remove(this)
        container.repaint(area.x, area.y, area.width, area.height)
    }
}
