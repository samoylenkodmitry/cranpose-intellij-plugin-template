package dev.cranpose.intellij

import com.intellij.openapi.Disposable
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.EditorFactory
import com.intellij.openapi.editor.event.CaretEvent
import com.intellij.openapi.editor.event.CaretListener
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerEvent
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import java.awt.Point
import java.awt.Rectangle

/**
 * Lays the process's `editor` overlay over the visible part of the selected
 * editor and reports that editor's caret changes on [CARET_CHANNEL], for the
 * effects in `ui/src/effects.rs`.
 */
class EditorOverlay(private val project: Project, private val panel: CranposePanel, private val parent: Disposable) {
    private var overlay: SurfaceOverlay? = null
    private var editor: Editor? = null
    private var attachment: Disposable? = null
    private var textLength = 0
    private var lastCaret: Point? = null

    init {
        panel.onOverlayOpened = { opened ->
            if (opened.anchor == ANCHOR) {
                overlay = opened
                attach(FileEditorManager.getInstance(project).selectedTextEditor)
            }
        }
        panel.onOverlayClosed = { closed ->
            if (closed === overlay) {
                detach()
                overlay = null
            }
        }
        project.messageBus.connect(parent).subscribe(
            FileEditorManagerListener.FILE_EDITOR_MANAGER,
            object : FileEditorManagerListener {
                override fun selectionChanged(event: FileEditorManagerEvent) =
                    attach(FileEditorManager.getInstance(project).selectedTextEditor)
            },
        )
        EditorFactory.getInstance().eventMulticaster.addCaretListener(
            object : CaretListener {
                override fun caretPositionChanged(event: CaretEvent) = caretMoved(event)
            },
            parent,
        )
    }

    private fun attach(target: Editor?) {
        val shown = overlay ?: return
        if (target === editor) return
        detach()
        target ?: return
        editor = target
        textLength = target.document.textLength
        target.contentComponent.add(shown)
        val scope = Disposer.newDisposable("Cranpose editor overlay").also { Disposer.register(parent, it) }
        target.scrollingModel.addVisibleAreaListener({ fit(target) }, scope)
        attachment = scope
        fit(target)
    }

    private fun fit(target: Editor) {
        val shown = overlay ?: return
        val area = target.scrollingModel.visibleArea
        if (shown.bounds != area) {
            shown.bounds = Rectangle(area)
            shown.revalidate()
        }
    }

    private fun detach() {
        attachment?.let(Disposer::dispose)
        attachment = null
        editor = null
        lastCaret = null
        overlay?.removeFromParent()
    }

    private fun caretMoved(event: CaretEvent) {
        val target = editor?.takeIf { it === event.editor } ?: return
        val length = target.document.textLength
        val kind = when {
            length > textLength -> "type"
            length < textLength -> "delete"
            else -> "move"
        }
        textLength = length
        val area = target.scrollingModel.visibleArea
        val to = target.logicalPositionToXY(event.newPosition)
        val from = target.logicalPositionToXY(event.oldPosition)
        if (kind == "move" && to == lastCaret) return
        lastCaret = to
        panel.send(CARET_CHANNEL, caretPayload(kind, to, from, area, target.lineHeight))
    }

    companion object {
        const val ANCHOR = "editor"
        const val CARET_CHANNEL = "ide.caret"

        /** A caret change as `ui/src/effects.rs` reads it, relative to the [visible] area. */
        fun caretPayload(kind: String, to: Point, from: Point, visible: Rectangle, lineHeight: Int): String = FlatJson.encode(
            "kind" to kind,
            "x" to to.x - visible.x,
            "y" to to.y - visible.y,
            "fromX" to from.x - visible.x,
            "fromY" to from.y - visible.y,
            "lineHeight" to lineHeight,
        )
    }
}
