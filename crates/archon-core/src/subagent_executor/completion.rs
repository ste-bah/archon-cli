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
                match crate::agents::memory::save_agent_memory(
                    &agent_type,
                    &content,
                    &title,
                    &meta.tags,
                    memory.as_ref(),
                    &project_path,
                    meta.memory_scope.as_ref(),
                ) {
                    Ok(_) => {
                        // #171 Part 6: the write this cache exists to not miss.
                        // Invalidating here (rather than relying on the TTL
                        // backstop) means a spawn immediately after a completion
                        // recalls the new row instead of the pre-write block.
                        self.recall_cache
                            .invalidate(&agent_type, meta.memory_scope.as_ref());
                    }
                    Err(e) => {
                        tracing::warn!(agent = %agent_type, error = %e, "failed to save agent memory");
                    }
                }
            }
        }
        // The agent's name, read BEFORE `cleanup_agent` strips it from the
        // registry a few lines below.
        let name = self.registered_name_for(&subagent_id).await;

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

        // Tell the lead. Without this a subagent ends into the background
        // task-notification path and the lead learns nothing — a failed agent
        // and a finished one are equally silent (#184 M6).
        //
        // Note a cancelled agent arrives here as `Err("subagent cancelled")`:
        // this path is handed a `Result<String, String>`, so it cannot tell
        // cancellation from failure. Reporting it as failed with that reason is
        // the honest reading of what it knows.
        let (status, detail) = match &result {
            Ok(text) => (
                archon_tools::send_message::AgentStatusKind::Completed,
                summarize(text),
            ),
            Err(reason) => (
                archon_tools::send_message::AgentStatusKind::Failed,
                Some(reason.clone()),
            ),
        };
        self.notify_lead(&subagent_id, name.as_deref(), status, detail.as_deref())
            .await;
    }

    /// The name this agent was registered under, if any.
    async fn registered_name_for(&self, subagent_id: &str) -> Option<String> {
        let mgr = self.subagent_manager.lock().await;
        mgr.get_status(subagent_id)
            .and_then(|info| info.request.subagent_type.clone())
    }

    /// Queue a status envelope for the lead to read at its next round.
    pub(super) async fn notify_lead(
        &self,
        subagent_id: &str,
        name: Option<&str>,
        status: archon_tools::send_message::AgentStatusKind,
        detail: Option<&str>,
    ) {
        let envelope = archon_tools::send_message::build_agent_status_envelope(
            subagent_id,
            name,
            status,
            detail,
        );

        let mut mgr = self.subagent_manager.lock().await;
        // Bounded like every other inbox. A storm of failing agents must not
        // grow the lead's queue without limit; dropping the newest and saying
        // so in the log beats an unbounded Vec nobody drains.
        if mgr.pending_message_count(crate::message_router::LEAD_QUEUE_ID)
            >= crate::message_router::MAX_PENDING_MESSAGES
        {
            tracing::warn!(
                subagent_id,
                "lead inbox is full; dropping an agent status envelope"
            );
            return;
        }
        mgr.queue_pending_message(crate::message_router::LEAD_QUEUE_ID, envelope);
    }
}

/// What an isolated agent left behind, and what to do with it (#184 M7).
///
/// The note used to be a path and a branch name. That tells the lead where the
/// work is but nothing about whether it is worth looking at, so a run of five
/// isolated agents produced five identical-looking notes and no way to triage
/// them. It now carries the diffstat, how far the branch diverged, and the
/// commands that act on it.
///
/// The review is best-effort: a repository that cannot be opened yields the
/// old shape rather than nothing, because knowing where the work is beats
/// knowing nothing.
fn preserved_worktree_note(wt: &archon_tools::worktree_manager::WorktreeInfo) -> String {
    let summary = match archon_tools::worktree_review::review_for(wt) {
        Some(review) => review.describe(),
        None => format!("branch '{}'", wt.branch_name),
    };
    let usage = archon_tools::worktree_manager::WorktreeManager::disk_usage(&wt.owner_id);

    format!(
        "\n\n[Worktree preserved: {summary}, {}]\n\
         Path: {}\n\
         Review with `/worktrees`, then merge or discard it — an isolated \
         agent's work is not in your tree until you say so.",
        usage.describe(),
        wt.worktree_path.display(),
    )
}

/// Trim a completion result down to something worth putting in an envelope.
///
/// The full text already reaches the lead as the tool result; the envelope is
/// a signal, not a second copy of the output.
fn summarize(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(280).collect())
}

impl AgentSubagentExecutor {
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
                        &archon_tools::worktree_ownership::subagent_owner_key(&subagent_id),
                    ) {
                        Ok(()) => {
                            tracing::info!(subagent_id = %subagent_id, "clean worktree auto-removed");
                        }
                        Err(_has_changes) => {
                            side_effects.text_suffix = Some(preserved_worktree_note(&wt));
                            tracing::info!(subagent_id = %subagent_id, branch = %wt.branch_name, "worktree preserved with changes");
                        }
                    }
                }
                Err(_) => {
                    // Silent cleanup on failure.
                    let _ = archon_tools::worktree_manager::WorktreeManager::cleanup_session(
                        &archon_tools::worktree_ownership::subagent_owner_key(&subagent_id),
                    );
                    tracing::info!(subagent_id = %subagent_id, "worktree cleaned up after failure");
                }
            }
        }

        side_effects
    }
}
