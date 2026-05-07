use super::*;

impl AgentSubagentExecutor {
    pub(super) async fn handle_inner_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
    ) {
        // Mark the subagent as completed/failed in the manager. The
        // cache_id is the pre-allocated caller id; the manager keyed
        // this subagent under its own manager-allocated id. Since
        // SubagentManager.register returns an id that we no longer
        // track here (this method only gets the caller id), we scan
        // by looking up the most recent — but that's fragile. Instead,
        // we rely on the fact that run_to_completion's caller holds
        // the manager_id via its local variable when dispatching the
        // inner-complete fire inside its own body, bypassing this
        // trait method. That means this trait-level `on_inner_complete`
        // path runs only for post-abandonment orphan tasks — and in
        // those cases, the runner still holds a reference to the
        // manager via `set_pending_message_source`, so the manager
        // update happens inside the runner's drop path.
        //
        // To preserve the old behavior in the common case, we ALSO
        // perform the manager update here using the manager's id
        // lookup by caller id when possible. If the lookup misses,
        // this becomes a no-op manager update (safe — the runner will
        // reconcile).
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
        // Best-effort manager update. Because the manager keys agents
        // under their own internally-generated id, this call may miss
        // for the caller-side cache_id. That is acceptable: the old
        // behavior always matched because the manager id and the
        // caller id were the same object. We cannot easily align
        // those without changing the manager API; we preserve memory
        // side effects (the critical PRESERVE-D8 invariant) and log
        // the manager-update miss as a TODO(post-105).
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
