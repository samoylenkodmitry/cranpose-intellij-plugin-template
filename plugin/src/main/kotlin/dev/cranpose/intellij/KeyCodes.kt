package dev.cranpose.intellij

import java.awt.event.InputEvent
import java.awt.event.KeyEvent

/** Translates AWT keyboard state into the names the Cranpose protocol uses. */
object KeyCodes {
    private val named: Map<Int, String> = buildMap {
        for (letter in 'A'..'Z') put(KeyEvent.VK_A + (letter - 'A'), "Key$letter")
        for (digit in 0..9) put(KeyEvent.VK_0 + digit, "Digit$digit")
        for (index in 1..12) put(KeyEvent.VK_F1 + index - 1, "F$index")
        put(KeyEvent.VK_UP, "ArrowUp")
        put(KeyEvent.VK_DOWN, "ArrowDown")
        put(KeyEvent.VK_LEFT, "ArrowLeft")
        put(KeyEvent.VK_RIGHT, "ArrowRight")
        put(KeyEvent.VK_HOME, "Home")
        put(KeyEvent.VK_END, "End")
        put(KeyEvent.VK_PAGE_UP, "PageUp")
        put(KeyEvent.VK_PAGE_DOWN, "PageDown")
        put(KeyEvent.VK_BACK_SPACE, "Backspace")
        put(KeyEvent.VK_DELETE, "Delete")
        put(KeyEvent.VK_ENTER, "Enter")
        put(KeyEvent.VK_TAB, "Tab")
        put(KeyEvent.VK_SPACE, "Space")
        put(KeyEvent.VK_ESCAPE, "Escape")
        put(KeyEvent.VK_MINUS, "Minus")
        put(KeyEvent.VK_EQUALS, "Equal")
        put(KeyEvent.VK_OPEN_BRACKET, "BracketLeft")
        put(KeyEvent.VK_CLOSE_BRACKET, "BracketRight")
        put(KeyEvent.VK_BACK_SLASH, "Backslash")
        put(KeyEvent.VK_SEMICOLON, "Semicolon")
        put(KeyEvent.VK_QUOTE, "Quote")
        put(KeyEvent.VK_COMMA, "Comma")
        put(KeyEvent.VK_PERIOD, "Period")
        put(KeyEvent.VK_SLASH, "Slash")
        put(KeyEvent.VK_BACK_QUOTE, "Backquote")
    }

    private val sided: Map<Int, String> = mapOf(
        KeyEvent.VK_SHIFT to "Shift",
        KeyEvent.VK_CONTROL to "Control",
        KeyEvent.VK_ALT to "Alt",
        KeyEvent.VK_META to "Meta",
    )

    /** The W3C `KeyboardEvent.code` for an AWT key, or `null` when it has none here. */
    fun domCode(keyCode: Int, location: Int): String? {
        sided[keyCode]?.let { side ->
            return side + if (location == KeyEvent.KEY_LOCATION_RIGHT) "Right" else "Left"
        }
        if (keyCode == KeyEvent.VK_ENTER && location == KeyEvent.KEY_LOCATION_NUMPAD) return "NumpadEnter"
        return named[keyCode]
    }

    /** The protocol's modifier bits for AWT extended modifiers. */
    fun modifiers(modifiersEx: Int): Int {
        var bits = 0
        if (modifiersEx and InputEvent.SHIFT_DOWN_MASK != 0) bits = bits or CranposeProtocol.MODIFIER_SHIFT
        if (modifiersEx and InputEvent.CTRL_DOWN_MASK != 0) bits = bits or CranposeProtocol.MODIFIER_CTRL
        if (modifiersEx and InputEvent.ALT_DOWN_MASK != 0) bits = bits or CranposeProtocol.MODIFIER_ALT
        if (modifiersEx and InputEvent.META_DOWN_MASK != 0) bits = bits or CranposeProtocol.MODIFIER_META
        return bits
    }

    /** Whether a typed character inserts text rather than acting as a control key or shortcut. */
    fun insertsText(char: Char, modifiersEx: Int): Boolean {
        val shortcut = InputEvent.CTRL_DOWN_MASK or InputEvent.META_DOWN_MASK
        return char != KeyEvent.CHAR_UNDEFINED && !char.isISOControl() && modifiersEx and shortcut == 0
    }
}
