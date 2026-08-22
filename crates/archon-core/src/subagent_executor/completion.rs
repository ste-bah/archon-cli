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

        // Vacate the team seat, if this agent held one (#184 M5). This is also
        // the acknowledgement `TeamDelete` waits on, so it has to happen on
        // every terminal state and not only on success.
        archon_tools::team_roster::leave(&subagent_id);

        // Release this agent's read-before-write observations (#193 Phase A).
        //
        // A session's observations are dropped when `SessionEnd` fires, which
        // is once per process. Subagents end constantly inside one — a plan
        // execution runs a fresh agent per task — and each holds its own map
        // keyed by its own id, so left here they accumulate for the life of the
        // process for agents that will never be consulted again.
        //
        // This site and not the `SubagentStop` hook, for the reason
        // `board/leases.rs` spells out: that hook fires from
        // `on_visible_complete`, which the `AutoBackgrounded` arm deliberately
        // skips, so the longest-running agents would never release. This
        // function is the one that always runs — success, failure and
        // cancellation all arrive here, including from the orphaned task of an
        // auto-backgrounded agent.
        //
        // Scoped to the agent, never `forget_session`: the parent is still
        // running and still holds readings behind edits it has not made yet.
        // Wiping those would turn `Fresh` into `Unobserved` and refuse a
        // legitimate write, which is a user-visible regression, not a tidy-up.
        //
        // The one visible consequence, stated rather than discovered: a stopped
        // agent can be restarted from its transcript under the same id
        // (`message_router::route_text` -> `RouterHost::resume_stopped_agent`),
        // and it comes back with no observations, so its first `Edit` to a file
        // it read before stopping is refused with "read it first" instead of
        // being allowed. That is the run boundary being applied consistently,
        // not an accident: `SubagentManager::register_with_id` already resets
        // the status, result, shutdown flag and progress tracker of a re-run,
        // and this function has already vacated the team seat and possibly
        // removed the worktree the agent was editing in. Observations were the
        // one piece of run state that outlived its run. A token minted before an
        // unbounded pause is exactly what this module's own header argues
        // against relying on, and the cost of not relying on it is one `Read`.
        archon_tools::file_observation::FILE_OBSERVATIONS.forget_agent(
            &archon_tools::file_observation::Observer::new(&self.session_id, Some(&subagent_id)),
        );

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use archon_llm::identity::{IdentityMode, IdentityProvider};
    use archon_llm::provider::{
        LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
    };
    use archon_llm::streaming::StreamEvent;
    use archon_tools::file_observation::{FILE_OBSERVATIONS, Observation, Observer};

    use super::*;
    use crate::agent::AgentConfig;
    use crate::agents::AgentRegistry;
    use crate::dispatch::ToolRegistry;
    use crate::subagent::SubagentManager;

    struct SilentProvider;

    #[async_trait::async_trait]
    impl LlmProvider for SilentProvider {
        fn name(&self) -> &str {
            "silent"
        }
        fn models(&self) -> Vec<ModelInfo> {
            vec![]
        }
        fn supports_feature(&self, _: ProviderFeature) -> bool {
            false
        }
        async fn stream(
            &self,
            _request: LlmRequest,
        ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }
        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
            unimplemented!()
        }
    }

    fn executor(session_id: &str) -> AgentSubagentExecutor {
        let project_dir = std::env::temp_dir();
        AgentSubagentExecutor::new(
            Arc::new(SilentProvider),
            ToolRegistry::new(),
            Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(&project_dir))),
            None,
            None,
            project_dir,
            session_id.to_string(),
            "claude-sonnet-4-6".into(),
            vec![],
            Arc::new(tokio::sync::Mutex::new("default".to_string())),
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            Arc::new(AgentConfig::default()),
            Arc::new(IdentityProvider::new(
                IdentityMode::Clean,
                session_id.to_string(),
                String::new(),
                String::new(),
            )),
        )
    }

    /// An agent that has ended must not still be holding what it read.
    ///
    /// This is the assertion that fails if the release is dropped from
    /// `handle_inner_complete`. It is deliberately about the *unconditional*
    /// completion path — every terminal state arrives here, including a
    /// cancellation and the orphaned task of an auto-backgrounded agent, which
    /// is why the release lives here and not on the `SubagentStop` hook.
    ///
    /// The second half is the other way this can be got wrong: reaching for
    /// `forget_session` looks equivalent and is not. The parent is still
    /// running, and taking its readings away turns `Fresh` into `Unobserved`,
    /// which under the default `read_before_edit = "block"` refuses a write it
    /// was entitled to make — at a moment decided by whichever child finished.
    #[tokio::test]
    async fn a_finished_subagent_releases_its_observations_and_only_its_own() {
        let session = format!("obs-release-{}", uuid::Uuid::new_v4());
        let executor = executor(&session);
        let path = std::env::temp_dir().join("observation-lifecycle-probe.rs");
        let version = archon_tools::file_observation::FileVersion::from_parts(1, Some(1));

        let child = Observer::new(&session, Some("agent-1"));
        let parent = Observer::new(&session, None);
        FILE_OBSERVATIONS.record_as(&child, &path, Observation::Present(version.clone()));
        FILE_OBSERVATIONS.record_as(&parent, &path, Observation::Present(version));

        executor
            .handle_inner_complete("agent-1".to_string(), Ok("done".to_string()))
            .await;

        assert!(
            FILE_OBSERVATIONS.is_empty(&child),
            "a subagent that reached its unconditional completion must not leave \
             its observations in the process-global map"
        );
        assert_eq!(
            FILE_OBSERVATIONS.len(&parent),
            1,
            "the still-running parent's readings are not the child's to drop"
        );
        FILE_OBSERVATIONS.forget_session(&session);
    }

    /// The same release on the failure arm.
    ///
    /// A cancelled agent arrives here as `Err`, and a cancelled agent is
    /// exactly the one that read a lot and finished nothing.
    #[tokio::test]
    async fn a_failed_subagent_releases_its_observations_too() {
        let session = format!("obs-release-fail-{}", uuid::Uuid::new_v4());
        let executor = executor(&session);
        let path = std::env::temp_dir().join("observation-lifecycle-probe.rs");

        let child = Observer::new(&session, Some("agent-2"));
        FILE_OBSERVATIONS.record_as(&child, &path, Observation::Absent);

        executor
            .handle_inner_complete("agent-2".to_string(), Err("subagent cancelled".to_string()))
            .await;

        assert!(FILE_OBSERVATIONS.is_empty(&child));
    }
}
