package dev.cranpose.intellij

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files
import java.nio.file.attribute.FileTime

class BinaryWatcherTest {
    @Test
    fun aChangeIsReportedOnceItSettles() {
        val binary = Files.createTempFile("cranpose-ui", "")
        Files.writeString(binary, "one")
        Files.setLastModifiedTime(binary, FileTime.fromMillis(1_000_000))
        val watcher = BinaryWatcher(binary)
        assertFalse(watcher.poll())

        Files.writeString(binary, "second build")
        Files.setLastModifiedTime(binary, FileTime.fromMillis(2_000_000))
        assertFalse("a file that just changed may still be written", watcher.poll())
        assertTrue(watcher.poll())
        assertFalse("each change is reported once", watcher.poll())
    }

    @Test
    fun aMissingFileIsNoChange() {
        val binary = Files.createTempFile("cranpose-ui", "")
        val watcher = BinaryWatcher(binary)
        Files.delete(binary)
        assertFalse(watcher.poll())
        assertFalse(watcher.poll())
    }
}
