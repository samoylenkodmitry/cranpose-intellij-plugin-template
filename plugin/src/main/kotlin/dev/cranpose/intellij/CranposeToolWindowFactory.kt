package dev.cranpose.intellij

import com.intellij.openapi.application.PathManager
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.project.DumbAware
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.content.ContentFactory
import java.nio.file.Path
import javax.swing.Timer

/** Opens the Cranpose UI in a tool window. */
class CranposeToolWindowFactory : ToolWindowFactory, DumbAware {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val override = UiBinary.override()
        val panel = CranposePanel(
            command = { listOf(binary(override).toString()) },
            log = { line -> LOG.info("[cranpose-ui] $line") },
        )
        val content = ContentFactory.getInstance().createContent(panel, "", false)
        toolWindow.contentManager.addContent(content)
        IdeBridge(project, panel, content)
        EditorOverlay(project, panel, content)
        Disposer.register(content) { panel.close() }
        override?.let { watch(it, panel, content) }
        panel.start()
    }

    private fun binary(override: Path?): Path =
        override ?: UiBinary.extractBundled(Path.of(PathManager.getSystemPath(), "cranpose-ui"))

    private fun watch(binary: Path, panel: CranposePanel, parent: com.intellij.openapi.Disposable) {
        val watcher = BinaryWatcher(binary)
        val timer = Timer(RELOAD_POLL_MILLIS) {
            if (watcher.poll()) {
                LOG.info("Cranpose UI binary changed, reloading")
                panel.start()
            }
        }
        timer.start()
        Disposer.register(parent) { timer.stop() }
    }

    private companion object {
        const val RELOAD_POLL_MILLIS = 700
        val LOG = logger<CranposeToolWindowFactory>()
    }
}
