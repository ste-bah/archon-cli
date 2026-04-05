package dev.archon.jetbrains.actions

import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys

/**
 * Editor popup action: sends selected code to Archon as a free-form question.
 *
 * The action is enabled only when there is an active text selection.
 * All connection I/O is off-EDT via [ConnectionManager].
 */
class AskArchonAction : AnAction("Ask Archon About This") {

    fun getText(): String = "Ask Archon About This"

    override fun actionPerformed(e: AnActionEvent) {
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return
        val selection = editor.selectionModel.selectedText ?: return
        val project = e.project ?: return
        // Phase 6: wire openChatWithContext(project, selection)
        // No-op until ChatToolWindow service is registered in the project service layer.
        @Suppress("UNUSED_EXPRESSION")
        selection
    }

    override fun update(e: AnActionEvent) {
        val editor = e.getData(CommonDataKeys.EDITOR)
        e.presentation.isEnabled = editor?.selectionModel?.hasSelection() == true
    }
}
