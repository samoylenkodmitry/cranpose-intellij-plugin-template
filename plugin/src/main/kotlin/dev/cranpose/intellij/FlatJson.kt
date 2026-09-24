package dev.cranpose.intellij

/**
 * The JSON the bridge channels carry: one object of string, number and boolean
 * fields, with no nesting. Small enough to keep the template free of a JSON
 * dependency.
 */
object FlatJson {
    /** Encodes [fields] as a JSON object, in the order given. */
    fun encode(vararg fields: Pair<String, Any>): String = fields.joinToString(",", "{", "}") { (key, value) ->
        val encoded = when (value) {
            is Boolean, is Int, is Long -> value.toString()
            is Number -> value.toDouble().toString()
            else -> quote(value.toString())
        }
        "${quote(key)}:$encoded"
    }

    /** The string fields of a flat JSON object; `null` when [json] is not one. */
    fun decodeStrings(json: String): Map<String, String>? {
        val parser = Parser(json)
        return runCatching { parser.objectOfStrings() }.getOrNull()
    }

    private fun quote(text: String): String = buildString {
        append('"')
        for (char in text) {
            when (char) {
                '"' -> append("\\\"")
                '\\' -> append("\\\\")
                '\n' -> append("\\n")
                '\r' -> append("\\r")
                '\t' -> append("\\t")
                else -> if (char < ' ') append("\\u%04x".format(char.code)) else append(char)
            }
        }
        append('"')
    }

    private class Parser(private val text: String) {
        private var at = 0

        fun objectOfStrings(): Map<String, String> {
            val fields = LinkedHashMap<String, String>()
            expect('{')
            skipSpace()
            if (peek() == '}') {
                at++
                return fields.also { end() }
            }
            while (true) {
                val key = string()
                expect(':')
                skipSpace()
                if (peek() == '"') fields[key] = string() else scalar()
                skipSpace()
                when (text.getOrNull(at++)) {
                    ',' -> Unit
                    '}' -> return fields.also { end() }
                    else -> error("expected , or } at ${at - 1}")
                }
            }
        }

        private fun string(): String {
            expect('"')
            val out = StringBuilder()
            while (true) {
                when (val char = text.getOrNull(at++) ?: error("unterminated string")) {
                    '"' -> return out.toString()
                    '\\' -> out.append(escape())
                    else -> out.append(char)
                }
            }
        }

        private fun escape(): Char = when (val char = text.getOrNull(at++) ?: error("unterminated escape")) {
            'n' -> '\n'
            'r' -> '\r'
            't' -> '\t'
            'b' -> '\b'
            'f' -> '\u000C'
            'u' -> text.substring(at, at + 4).toInt(16).toChar().also { at += 4 }
            else -> char
        }

        private fun scalar() {
            while (at < text.length && text[at] !in ",}") at++
        }

        private fun expect(char: Char) {
            skipSpace()
            if (text.getOrNull(at) != char) error("expected $char at $at")
            at++
        }

        private fun end() {
            skipSpace()
            if (at != text.length) error("trailing text at $at")
        }

        private fun peek(): Char? = text.getOrNull(at)

        private fun skipSpace() {
            while (at < text.length && text[at].isWhitespace()) at++
        }
    }
}
