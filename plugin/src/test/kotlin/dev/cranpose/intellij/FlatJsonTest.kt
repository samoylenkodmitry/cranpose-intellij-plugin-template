package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class FlatJsonTest {
    @Test
    fun fieldsEncodeInOrderWithEscapes() {
        assertEquals(
            """{"dark":true,"size":13,"name":"a \"b\"\n\\c"}""",
            FlatJson.encode("dark" to true, "size" to 13, "name" to "a \"b\"\n\\c"),
        )
    }

    @Test
    fun stringFieldsDecodeAndOtherValuesAreSkipped() {
        assertEquals(
            mapOf("title" to "Hi \"you\"", "content" to "é\n"),
            FlatJson.decodeStrings("""{ "title" : "Hi \"you\"", "count": 3, "ok": false, "content": "é\n" }"""),
        )
        assertEquals(emptyMap<String, String>(), FlatJson.decodeStrings("{}"))
    }

    @Test
    fun malformedJsonIsRejected() {
        assertNull(FlatJson.decodeStrings("""{"title": "unterminated}"""))
        assertNull(FlatJson.decodeStrings("""{"a":"b"} trailing"""))
        assertNull(FlatJson.decodeStrings("[1,2]"))
    }

    @Test
    fun whatIsEncodedDecodes() {
        val encoded = FlatJson.encode("path" to "/tmp/a \"b\".rs", "name" to "tab\there")
        assertEquals(mapOf("path" to "/tmp/a \"b\".rs", "name" to "tab\there"), FlatJson.decodeStrings(encoded))
    }
}
