//! Resumes reserve the same resolved objects atomically with their manager id.
use super::*;
use crate::subagent::runner::EffectiveRunContext;
use archon_tools::isolation::IsolationTier;

impl AgentSubagentExecutor {
    pub(super) fn resume_context(
        &self,
        manager: &SubagentManager,
        id: &str,
        pending: Option<&crate::agents::transcript::PendingResume>,
        continuing: bool,
    ) -> Result<Option<Arc<EffectiveRunContext>>, ExecutorError> {
        // Only a message resume reaches past here; it runs exactly or refuses.
        let Some(pending) = pending else {
            return match continuing {
                true => self.repair_context(manager, id),
                false => Ok(None),
            };
        };
        let info = manager.get_status(id).ok_or_else(|| {
            ExecutorError::Internal(crate::agents::transcript::resume::unknown_context(id))
        })?;
        if pending.agent_id != id || pending.generation != info.generation {
            return Err(ExecutorError::Internal(format!(
                "cannot resume agent '{id}': its pending resume was superseded; start a new agent"
            )));
        }
        let context = info.effective_context.clone().ok_or_else(|| {
            ExecutorError::Internal(crate::agents::transcript::resume::unknown_context(id))
        })?;
        check_rung(
            id,
            context.tier,
            self.agent_config.subagent_isolation_max_tier,
        )
        .map_err(ExecutorError::Internal)?;
        Ok(Some(context))
    }

    /// The stored context a workflow's validation repair continues with;
    /// `None` to run it as a clean re-run of the workflow's call; `Err` to
    /// refuse it.
    ///
    /// A repair is the workflow's call: the workflow passes its original
    /// request and its own context again. Only two losses make it a clean
    /// re-run: the context is gone (another process started the agent, or it
    /// was collected), or the worktree it ran in was removed at completion.
    /// A clean re-run is a new call of the workflow's definition, so it keeps
    /// only the workflow's own prompt from the history and none of what the
    /// earlier agent did. Any other difference (a changed sandbox, a lowered
    /// tier cap, a cancelled scope, a missing directory) refuses: the run
    /// would be weaker than the one the history came from, and the workflow
    /// already handles a failed repair.
    fn repair_context(
        &self,
        manager: &SubagentManager,
        id: &str,
    ) -> Result<Option<Arc<EffectiveRunContext>>, ExecutorError> {
        let Some(context) = manager
            .get_status(id)
            .and_then(|info| info.effective_context.clone())
        else {
            tracing::warn!(subagent_id = %id, "validation repair has no stored context; clean re-run of the workflow's call");
            return Ok(None);
        };
        if context.worktree_removed() {
            tracing::warn!(subagent_id = %id, "validation repair's worktree was removed; clean re-run of the workflow's call");
            return Ok(None);
        }
        context
            .usable(id)
            .and_then(|()| check_rung(id, context.tier, self.agent_config.subagent_isolation_max_tier))
            .map_err(ExecutorError::Internal)?;
        Ok(Some(context))
    }

    pub(super) async fn refuse_run(&self, manager_id: &str, reason: String) -> ExecutorError {
        let _ = self
            .subagent_manager
            .lock()
            .await
            .mark_failed(manager_id, reason.clone());
        ExecutorError::Internal(reason)
    }

    pub(super) async fn restored_runner(
        &self,
        ids: &super::run_prepare::RunIdentity,
        context: &EffectiveRunContext,
        cancel: &CancellationToken,
        caller: Option<&CancellationToken>,
    ) -> Result<super::run_runner::BuiltRunner, ExecutorError> {
        let mut runner = context
            .runner(&ids.manager_id, cancel, caller)
            .map_err(ExecutorError::Internal)?;
        let tool_cancellation = runner.tool_cancellation();
        if let Some(worktree) = &context.worktree {
            self.worktree_cache
                .lock()
                .await
                .insert(ids.cache_id.clone(), worktree.clone());
        }
        if let Some(store) = crate::agents::transcript::AgentTranscriptStore::new(&self.session_id)
        {
            runner.set_transcript(store, ids.manager_id.clone());
        }
        self.configure_resume_and_progress(&mut runner, &ids.manager_id, true)
            .await;
        if let Some(messages) = &ids.resume_messages {
            runner.set_initial_messages(messages.clone());
        }
        if let Some(session) = archon_tools::subagent_session::current_for(&ids.manager_id)
            && session.continuing
            && session
                .history
                .messages()
                .last()
                .and_then(|message| message["role"].as_str())
                != Some("assistant")
        {
            return Err(ExecutorError::Internal(
                "validation repair has no completed assistant history".into(),
            ));
        }
        Ok(super::run_runner::BuiltRunner {
            runner,
            request: context.request.clone(),
            worktree: context.worktree.clone(),
            tool_cancellation,
        })
    }
}

/// Pin to the stored rung. A current cap can refuse it, never lower it.
pub(super) fn check_rung(
    id: &str,
    stored: IsolationTier,
    cap: IsolationTier,
) -> Result<(), String> {
    if stored > cap {
        return Err(format!(
            "cannot resume agent '{id}': it ran on the isolation rung '{}', but subagent.isolation_max_tier now permits only '{}'; raise the cap or start a new agent",
            stored.as_str(),
            cap.as_str()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "run_resume_tests.rs"]
mod tests;
