use super::*;

impl AgentSubagentExecutor {
    pub(super) async fn handle_inner_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
    ) {
        // Save agent memory (PRESERVE-D8 — single collapsed site).
        if let Ok(ref text) = result {
            let meta = self.memory_cache.lock().await.get(&subagent_id).cloned();
            if let Some(meta) = meta
                && let (Some(agent_type), Some(memory)) = (meta.agent_type, self.memory.as_ref())
            {
                let content: String = text.chars().take(500).collect();
                let title = format!("completion:{}:{}", agent_type, subagent_id);
                let project_path = self.working_dir.to_string_lossy();
                if let Err(e) = crate::agents::memory::save_agent_memory(
                    &agent_type,
                    &content,
                    &title,
                    &meta.tags,
                    memory.as_ref(),
                    &project_path,
                    meta.memory_scope.as_ref(),
                ) {
                    tracing::warn!(agent = %agent_type, error = %e, "failed to save agent memory");
                }
            }
        }
        // Best-effort manager update. The caller id is now the manager id,
        // so visible status, SendMessage, progress, transcripts, and cleanup
        // all converge on the same identifier.
        match &result {
            Ok(text) => {
                let mut mgr = self.subagent_manager.lock().await;
                let _ = mgr.complete(&subagent_id, text.clone());
                mgr.cleanup_agent(&subagent_id);
            }
            Err(reason) => {
                let mut mgr = self.subagent_manager.lock().await;
                let _ = mgr.mark_failed(&subagent_id, reason.clone());
                mgr.cleanup_agent(&subagent_id);
            }
        }
    }

    pub(super) async fn handle_visible_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
        nested: bool,
    ) -> OutcomeSideEffects {
        let mut side_effects = OutcomeSideEffects::default();

        // Hook fires: collapsed from H3+H7 / H4+H8 / H5+H9 / H6+H10.
        match &result {
            Ok(_) => {
                self.fire_hook(
                    HookEvent::TeammateIdle,
                    serde_json::json!({
                        "hook_event": "TeammateIdle",
                        "subagent_id": subagent_id,
                    }),
                )
                .await;
                self.fire_hook(
                    HookEvent::SubagentStop,
                    serde_json::json!({
                        "hook_event": "SubagentStop",
                        "subagent_id": subagent_id,
                        "success": true,
                    }),
                )
                .await;
                if nested {
                    self.fire_hook(
                        HookEvent::TaskCompleted,
                        serde_json::json!({
                            "hook_event": "TaskCompleted",
                            "subagent_id": subagent_id,
                            "success": true,
                        }),
                    )
                    .await;
                }
            }
            Err(reason) => {
                self.fire_hook(
                    HookEvent::SubagentStop,
                    serde_json::json!({
                        "hook_event": "SubagentStop",
                        "subagent_id": subagent_id,
                        "success": false,
                        "error": reason,
                    }),
                )
                .await;
            }
        }

        // Worktree cleanup: consume the cached worktree_info (if any).
        let wt_entry = self.worktree_cache.lock().await.remove(&subagent_id);
        if let Some(wt) = wt_entry {
            match &result {
                Ok(_) => {
                    // Clean vs. has_changes split.
                    match archon_tools::worktree_manager::WorktreeManager::cleanup_session(
                        &format!("subagent-{subagent_id}"),
                    ) {
                        Ok(()) => {
                            tracing::info!(subagent_id = %subagent_id, "clean worktree auto-removed");
                        }
                        Err(_has_changes) => {
                            let wt_note = format!(
                                "\n\n[Worktree: {} (branch: {})]",
                                wt.worktree_path.display(),
                                wt.branch_name
                            );
                            side_effects.text_suffix = Some(wt_note);
                            tracing::info!(subagent_id = %subagent_id, branch = %wt.branch_name, "worktree preserved with changes");
                        }
                    }
                }
                Err(_) => {
                    // Silent cleanup on failure.
                    let _ = archon_tools::worktree_manager::WorktreeManager::cleanup_session(
                        &format!("subagent-{subagent_id}"),
                    );
                    tracing::info!(subagent_id = %subagent_id, "worktree cleaned up after failure");
                }
            }
        }

        side_effects
    }
}
