package dev.archon.jetbrains

import com.intellij.openapi.components.ProjectComponent
import com.intellij.openapi.project.Project

/**
 * Top-level project component.
 *
 * Lifecycle hooks are intentionally thin at this stage.
 * Phase 6 will wire auto-connect on [projectOpened] based on persisted settings.
 */
@Suppress("DEPRECATION") // ProjectComponent is still supported through IC 2024.x
class ArchonPlugin(private val project: Project) : ProjectComponent {

    override fun getComponentName(): String = Constants.PLUGIN_ID

    override fun projectOpened() {
        // Phase 6: read ArchonSettings, auto-connect if configured
    }

    override fun projectClosed() {
        // Phase 6: cleanly disconnect any active ConnectionManager
    }
}
