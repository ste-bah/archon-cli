package dev.archon.jetbrains.diff

import com.intellij.diff.DiffContentFactory
import com.intellij.diff.DiffManager
import com.intellij.diff.contents.DocumentContent
import com.intellij.diff.requests.SimpleDiffRequest
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile

/**
 * Shows a side-by-side diff between the current file contents and the code
 * proposed by Archon, using the standard JetBrains diff viewer.
 *
 * Must be called on the EDT; the diff viewer manages its own threading.
 */
object ArchonDiffView {
    fun showDiff(
        project: Project,
        @Suppress("UNUSED_PARAMETER") file: VirtualFile,
        originalContent: String,
        proposedContent: String
    ) {
        val original: DocumentContent =
            DiffContentFactory.getInstance().create(project, originalContent)
        val proposed: DocumentContent =
            DiffContentFactory.getInstance().create(project, proposedContent)
        val request = SimpleDiffRequest(
            "Archon Proposed Changes",
            original,
            proposed,
            "Original",
            "Archon"
        )
        DiffManager.getInstance().showDiff(project, request)
    }
}
