package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Test
import java.awt.Dimension
import java.awt.Rectangle

class SurfaceInputTest {
    private val start = Rectangle(100, 100, 200, 150)

    @Test
    fun eachEdgeMovesOnlyItsOwnSides() {
        assertEquals(Rectangle(100, 100, 230, 150), SurfaceInput.resized(start, ResizeEdge.EAST, 30, 20, null))
        assertEquals(Rectangle(70, 100, 230, 150), SurfaceInput.resized(start, ResizeEdge.WEST, -30, 20, null))
        assertEquals(Rectangle(100, 80, 200, 170), SurfaceInput.resized(start, ResizeEdge.NORTH, 30, -20, null))
        assertEquals(Rectangle(100, 100, 200, 170), SurfaceInput.resized(start, ResizeEdge.SOUTH, 30, 20, null))
        assertEquals(Rectangle(100, 100, 230, 170), SurfaceInput.resized(start, ResizeEdge.SOUTH_EAST, 30, 20, null))
        assertEquals(Rectangle(70, 80, 230, 170), SurfaceInput.resized(start, ResizeEdge.NORTH_WEST, -30, -20, null))
        assertEquals(Rectangle(100, 80, 230, 170), SurfaceInput.resized(start, ResizeEdge.NORTH_EAST, 30, -20, null))
        assertEquals(Rectangle(70, 100, 230, 170), SurfaceInput.resized(start, ResizeEdge.SOUTH_WEST, -30, 20, null))
    }

    @Test
    fun aWindowStopsShrinkingAtItsMinimumWithTheOppositeEdgeFixed() {
        assertEquals(Rectangle(252, 100, 48, 150), SurfaceInput.resized(start, ResizeEdge.WEST, 500, 0, null))
        assertEquals(Rectangle(100, 170, 200, 80), SurfaceInput.resized(start, ResizeEdge.NORTH, 0, 500, Dimension(10, 80)))
    }
}
