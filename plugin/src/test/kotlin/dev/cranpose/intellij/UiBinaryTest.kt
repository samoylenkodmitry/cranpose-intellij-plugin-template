package dev.cranpose.intellij

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path

class UiBinaryTest {
    @Test
    fun platformsNameTheBundleDirectory() {
        assertEquals("macos-aarch64", UiBinary.platform("Mac OS X", "aarch64"))
        assertEquals("linux-x86_64", UiBinary.platform("Linux", "amd64"))
        assertEquals("windows-x86_64", UiBinary.platform("Windows 11", "amd64"))
        assertEquals("windows-aarch64", UiBinary.platform("Windows 11", "arm64"))
        assertNull(UiBinary.platform("FreeBSD", "amd64"))
        assertNull(UiBinary.platform("Linux", "riscv64"))
    }

    @Test
    fun windowsExecutablesCarryTheirExtension() {
        assertEquals("cranpose-intellij-ui.exe", UiBinary.executableName("Windows 10"))
        assertEquals("cranpose-intellij-ui", UiBinary.executableName("Linux"))
        assertEquals("/native/linux-aarch64/cranpose-intellij-ui", UiBinary.resourcePath("Linux", "aarch64"))
        assertNull(UiBinary.resourcePath("SunOS", "sparc"))
    }

    @Test(expected = IOException::class)
    fun anUnbundledPlatformFailsToExtract() {
        UiBinary.extractBundled(Files.createTempDirectory("cranpose-ui"), "Linux", "riscv64")
    }

    @Test
    fun theOverridePropertyNamesTheDevelopersBuild() {
        val configured = System.getProperty(UiBinary.OVERRIDE_PROPERTY)
        assumeTrue("the test task sets ${UiBinary.OVERRIDE_PROPERTY}", configured != null)
        assertEquals(Path.of(configured), UiBinary.override())
    }
}
