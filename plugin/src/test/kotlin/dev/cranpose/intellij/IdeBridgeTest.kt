package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Test
import java.awt.Color
import java.awt.Cursor

class IdeBridgeTest {
    @Test
    fun colorsBecomeHex() {
        assertEquals("#2b2d30", IdeBridge.hex(Color(0x2B, 0x2D, 0x30)))
    }

    @Test
    fun cardsAreRaisedTowardTheOppositeOfTheTheme() {
        assertEquals(Color(0x39, 0x3B, 0x3E), IdeBridge.raised(Color(0x2B, 0x2D, 0x30), dark = true))
        assertEquals(Color(0xF4, 0xF4, 0xF4), IdeBridge.raised(Color(0xFF, 0xFF, 0xFF), dark = false))
    }

    @Test
    fun cssCursorsMapToAwtCursors() {
        assertEquals(Cursor.HAND_CURSOR, CranposePanel.cursorType("pointer"))
        assertEquals(Cursor.TEXT_CURSOR, CranposePanel.cursorType("text"))
        assertEquals(Cursor.E_RESIZE_CURSOR, CranposePanel.cursorType("ew-resize"))
        assertEquals(Cursor.DEFAULT_CURSOR, CranposePanel.cursorType("default"))
        assertEquals(Cursor.DEFAULT_CURSOR, CranposePanel.cursorType("no-such-cursor"))
    }
}
