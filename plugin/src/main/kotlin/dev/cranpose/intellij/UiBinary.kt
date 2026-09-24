package dev.cranpose.intellij

import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.HexFormat

/**
 * Where the Cranpose UI binary comes from: a path a developer points at with
 * [OVERRIDE_PROPERTY] or [OVERRIDE_VARIABLE] (the plugin then reloads the UI
 * whenever that file changes), or the build for this OS and architecture the
 * plugin bundles under `/native/<platform>/`.
 */
object UiBinary {
    const val NAME = "cranpose-intellij-ui"
    const val OVERRIDE_PROPERTY = "cranpose.ui.binary"
    const val OVERRIDE_VARIABLE = "CRANPOSE_UI_BINARY"

    /** The developer's own build, when one is configured. */
    fun override(): Path? = (System.getProperty(OVERRIDE_PROPERTY) ?: System.getenv(OVERRIDE_VARIABLE))
        ?.takeIf { it.isNotBlank() }
        ?.let(Path::of)

    /** The bundle directory name for an OS and architecture, such as `macos-aarch64`; `null` when unsupported. */
    fun platform(osName: String, osArch: String): String? {
        val os = when {
            osName.startsWith("Mac", ignoreCase = true) -> "macos"
            osName.startsWith("Windows", ignoreCase = true) -> "windows"
            osName.startsWith("Linux", ignoreCase = true) -> "linux"
            else -> return null
        }
        val arch = when (osArch.lowercase()) {
            "aarch64", "arm64" -> "aarch64"
            "amd64", "x86_64", "x64" -> "x86_64"
            else -> return null
        }
        return "$os-$arch"
    }

    /** The executable's file name on an OS. */
    fun executableName(osName: String): String =
        if (osName.startsWith("Windows", ignoreCase = true)) "$NAME.exe" else NAME

    /** The resource the bundled executable for an OS and architecture lives at. */
    fun resourcePath(osName: String, osArch: String): String? =
        platform(osName, osArch)?.let { "/native/$it/${executableName(osName)}" }

    /**
     * Copies the bundled executable for this machine into [cacheRoot], under a
     * directory named by its content hash so an updated plugin never runs a
     * stale copy, and returns its path.
     */
    fun extractBundled(
        cacheRoot: Path,
        osName: String = System.getProperty("os.name"),
        osArch: String = System.getProperty("os.arch"),
    ): Path {
        val resource = resourcePath(osName, osArch)
            ?: throw IOException("Cranpose has no build for $osName on $osArch")
        val bytes = UiBinary::class.java.getResourceAsStream(resource)?.use { it.readAllBytes() }
            ?: throw IOException("this plugin build does not bundle $resource")
        val hash = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)).take(16)
        val target = cacheRoot.resolve(hash).resolve(executableName(osName))
        if (!Files.exists(target) || Files.size(target) != bytes.size.toLong()) {
            Files.createDirectories(target.parent)
            val partial = Files.createTempFile(target.parent, NAME, ".partial")
            Files.write(partial, bytes)
            Files.move(partial, target, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
        }
        target.toFile().setExecutable(true)
        return target
    }
}
