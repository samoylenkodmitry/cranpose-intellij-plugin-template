package dev.cranpose.intellij

import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.attribute.FileTime

/**
 * Notices a rebuilt binary: [poll] reports a change once the file's time and
 * size have moved and then held still for one poll, so a build still writing
 * the file is never started half-written.
 */
class BinaryWatcher(private val path: Path) {
    private data class Stamp(val modified: FileTime, val size: Long)

    private var running: Stamp? = stamp()
    private var pending: Stamp? = null

    /** Whether the binary changed and settled since the last change was reported. */
    fun poll(): Boolean {
        val now = stamp() ?: return false
        if (now == running) {
            pending = null
            return false
        }
        if (now != pending) {
            pending = now
            return false
        }
        running = now
        pending = null
        return true
    }

    private fun stamp(): Stamp? = runCatching {
        Stamp(Files.getLastModifiedTime(path), Files.size(path))
    }.getOrNull()
}
