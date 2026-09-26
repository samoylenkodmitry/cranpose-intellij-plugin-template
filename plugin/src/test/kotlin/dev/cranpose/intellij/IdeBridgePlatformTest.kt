package dev.cranpose.intellij

import com.intellij.testFramework.fixtures.BasePlatformTestCase
import com.intellij.openapi.util.Disposer
import com.intellij.util.ui.UIUtil

class IdeBridgePlatformTest : BasePlatformTestCase() {
    fun testApplicationChannelsAcceptNestedPayloadsBeforeDefaultParsing() {
        val panel = CranposePanel({ emptyList() })
        Disposer.register(testRootDisposable) { panel.close() }
        var received = ""
        var ready = 0
        IdeBridge(project, panel, testRootDisposable, { ready++ }, { channel, payload ->
            if (channel == "custom.request") { received = payload; true } else false
        })
        panel.onConnected()
        assertEquals(1, ready)
        val nested = """{"targets":[{"name":"demo"}]}"""
        panel.onAppMessage("custom.request", nested)
        UIUtil.dispatchAllInvocationEvents()
        assertEquals(nested, received)
    }
}
