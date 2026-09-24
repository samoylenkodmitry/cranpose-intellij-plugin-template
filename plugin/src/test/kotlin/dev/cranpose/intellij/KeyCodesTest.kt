package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.awt.event.InputEvent
import java.awt.event.KeyEvent

class KeyCodesTest {
    @Test
    fun keysMapToDomCodes() {
        assertEquals("KeyA", KeyCodes.domCode(KeyEvent.VK_A, KeyEvent.KEY_LOCATION_STANDARD))
        assertEquals("Digit7", KeyCodes.domCode(KeyEvent.VK_7, KeyEvent.KEY_LOCATION_STANDARD))
        assertEquals("F12", KeyCodes.domCode(KeyEvent.VK_F12, KeyEvent.KEY_LOCATION_STANDARD))
        assertEquals("ArrowLeft", KeyCodes.domCode(KeyEvent.VK_LEFT, KeyEvent.KEY_LOCATION_STANDARD))
        assertEquals("Backspace", KeyCodes.domCode(KeyEvent.VK_BACK_SPACE, KeyEvent.KEY_LOCATION_STANDARD))
        assertEquals("Backquote", KeyCodes.domCode(KeyEvent.VK_BACK_QUOTE, KeyEvent.KEY_LOCATION_STANDARD))
    }

    @Test
    fun modifierKeysAndTheNumpadEnterKeepTheirSide() {
        assertEquals("ShiftLeft", KeyCodes.domCode(KeyEvent.VK_SHIFT, KeyEvent.KEY_LOCATION_LEFT))
        assertEquals("MetaRight", KeyCodes.domCode(KeyEvent.VK_META, KeyEvent.KEY_LOCATION_RIGHT))
        assertEquals("NumpadEnter", KeyCodes.domCode(KeyEvent.VK_ENTER, KeyEvent.KEY_LOCATION_NUMPAD))
        assertNull(KeyCodes.domCode(KeyEvent.VK_PRINTSCREEN, KeyEvent.KEY_LOCATION_STANDARD))
    }

    @Test
    fun modifiersBecomeProtocolBits() {
        assertEquals(0, KeyCodes.modifiers(0))
        assertEquals(
            CranposeProtocol.MODIFIER_SHIFT or CranposeProtocol.MODIFIER_META,
            KeyCodes.modifiers(InputEvent.SHIFT_DOWN_MASK or InputEvent.META_DOWN_MASK),
        )
        assertEquals(
            CranposeProtocol.MODIFIER_CTRL or CranposeProtocol.MODIFIER_ALT,
            KeyCodes.modifiers(InputEvent.CTRL_DOWN_MASK or InputEvent.ALT_DOWN_MASK),
        )
    }

    @Test
    fun onlyPrintableCharactersWithoutShortcutsInsertText() {
        assertTrue(KeyCodes.insertsText('a', 0))
        assertTrue(KeyCodes.insertsText('é', InputEvent.ALT_DOWN_MASK))
        assertFalse(KeyCodes.insertsText('\b', 0))
        assertFalse(KeyCodes.insertsText('c', InputEvent.META_DOWN_MASK))
        assertFalse(KeyCodes.insertsText(KeyEvent.CHAR_UNDEFINED, 0))
    }
}
