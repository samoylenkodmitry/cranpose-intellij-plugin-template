package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Test
import java.awt.Point
import java.awt.Rectangle

class EditorOverlayTest {
    @Test
    fun caretPositionsAreRelativeToTheVisibleArea() {
        assertEquals(
            """{"kind":"type","x":40,"y":16,"fromX":30,"fromY":16,"lineHeight":18}""",
            EditorOverlay.caretPayload("type", Point(50, 216), Point(40, 216), Rectangle(10, 200, 600, 400), 18),
        )
    }
}
