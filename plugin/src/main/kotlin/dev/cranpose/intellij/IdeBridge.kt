package dev.cranpose.intellij

import com.intellij.ide.ui.LafManagerListener
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.logger
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerEvent
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.ui.JBColor
import com.intellij.util.ui.JBUI
import com.intellij.util.ui.UIUtil
import java.awt.Color

/**
 * The IDE half of the message contract in `ui/src/ide.rs`: pushes the theme
 * and the focused editor to the UI, and carries out what the UI asks for.
 */
class IdeBridge(private val project: Project, private val panel: CranposePanel, parent: Disposable) {
    init {
        panel.onConnected = {
            sendTheme()
            sendEditor(FileEditorManager.getInstance(project).selectedFiles.firstOrNull())
        }
        panel.onAppMessage = { channel, payload ->
            ApplicationManager.getApplication().invokeLater({ handle(channel, payload) }, project.disposed)
        }
        ApplicationManager.getApplication().messageBus.connect(parent)
            .subscribe(LafManagerListener.TOPIC, LafManagerListener { sendTheme() })
        project.messageBus.connect(parent).subscribe(
            FileEditorManagerListener.FILE_EDITOR_MANAGER,
            object : FileEditorManagerListener {
                override fun selectionChanged(event: FileEditorManagerEvent) = sendEditor(event.newFile)
            },
        )
    }

    private fun sendTheme() {
        val dark = !JBColor.isBright()
        val background = JBColor.namedColor("ToolWindow.background", UIUtil.getPanelBackground())
        val accent = JBColor.namedColor("Button.default.startBackground", JBUI.CurrentTheme.Focus.focusColor())
        panel.background = background
        panel.setTheme(dark)
        panel.send(
            THEME_CHANNEL,
            FlatJson.encode(
                "dark" to dark,
                "background" to hex(background),
                "surface" to hex(raised(background, dark)),
                "text" to hex(UIUtil.getLabelForeground()),
                "muted" to hex(JBColor.namedColor("Label.infoForeground", JBColor.GRAY)),
                "accent" to hex(accent),
            ),
        )
    }

    private fun sendEditor(file: VirtualFile?) {
        val payload = file?.let { FlatJson.encode("path" to it.path, "name" to it.presentableName) } ?: "{}"
        panel.send(EDITOR_CHANNEL, payload)
    }

    private fun handle(channel: String, payload: String) {
        val fields = FlatJson.decodeStrings(payload)
        if (fields == null) {
            LOG.warn("Cranpose sent unreadable JSON on $channel: $payload")
            return
        }
        when (channel) {
            NOTIFY_CHANNEL -> NotificationGroupManager.getInstance()
                .getNotificationGroup(NOTIFICATION_GROUP)
                .createNotification(fields["title"] ?: "Cranpose", fields["content"].orEmpty(), NotificationType.INFORMATION)
                .notify(project)
            OPEN_CHANNEL -> fields["path"]
                ?.let { LocalFileSystem.getInstance().findFileByPath(it) }
                ?.let { FileEditorManager.getInstance(project).openFile(it, true) }
            else -> LOG.info("Cranpose sent a message on unknown channel $channel")
        }
    }

    companion object {
        const val THEME_CHANNEL = "ide.theme"
        const val EDITOR_CHANNEL = "ide.editor"
        const val NOTIFY_CHANNEL = "ide.notify"
        const val OPEN_CHANNEL = "ide.open"
        const val NOTIFICATION_GROUP = "Cranpose"

        private val LOG = logger<IdeBridge>()

        /** A color as `#rrggbb`. */
        fun hex(color: Color): String = "#%02x%02x%02x".format(color.red, color.green, color.blue)

        /** A shade of [background] that reads as a raised card on it. */
        fun raised(background: Color, dark: Boolean): Color {
            val toward = if (dark) 255 else 0
            val amount = if (dark) 0.07 else 0.04
            fun mix(channel: Int) = (channel + (toward - channel) * amount).toInt().coerceIn(0, 255)
            return Color(mix(background.red), mix(background.green), mix(background.blue))
        }
    }
}
