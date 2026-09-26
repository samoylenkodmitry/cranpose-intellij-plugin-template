package dev.cranpose.intellij

import org.junit.Assert.*
import org.junit.Test
import java.awt.event.MouseEvent
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import javax.swing.SwingUtilities

class PanelLifecycleTest {
    @Test fun aClosedUiProcessCanBeRestartedByClickingThePanel() {
        val binary = requireNotNull(System.getProperty(UiBinary.OVERRIDE_PROPERTY))
        val connections = AtomicInteger()
        val restarted = CountDownLatch(1)
        lateinit var panel: CranposePanel
        SwingUtilities.invokeAndWait {
            panel = CranposePanel({ listOf(binary) })
            panel.setSize(480, 640)
            panel.onConnected = {
                if (connections.incrementAndGet() == 1) panel.link.host?.close()
                else restarted.countDown()
            }
            panel.start()
        }
        try {
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(20)
            while ((connections.get() == 0 || panel.link.host != null) && System.nanoTime() < deadline) Thread.sleep(25)
            assertEquals(1, connections.get())
            assertNull("the disconnected host should be cleared", panel.link.host)
            SwingUtilities.invokeAndWait {
                panel.dispatchEvent(MouseEvent(panel, MouseEvent.MOUSE_PRESSED, System.currentTimeMillis(), 0, 20, 20, 1, false, MouseEvent.BUTTON1))
            }
            assertTrue("click should restart the disconnected UI", restarted.await(10, TimeUnit.SECONDS))
        } finally {
            SwingUtilities.invokeAndWait { panel.close() }
        }
    }
}
