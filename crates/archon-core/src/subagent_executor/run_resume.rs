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
            return Ok(continuing
                .then(|| self.repair_context(manager, id))
                .flatten());
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

    /// The stored context a workflow's validation repair continues with, or
    /// `None` to run it as a new agent of the workflow's own call.
    ///
    /// A repair is the workflow's call, not an agent's resume: the workflow
    /// passes the original request and its own context again, and the history
    /// is the in-memory completed history of that call, never a transcript.
    /// So a context that is gone (another process started the agent, or it
    /// was collected) or can no longer run exactly (its clean worktree was
    /// removed at completion, its sandbox or tier cap changed) does not stop
    /// the run: the workflow's definition decides the confinement again, as
    /// it did for the first call.
    fn repair_context(
        &self,
        manager: &SubagentManager,
        id: &str,
    ) -> Option<Arc<EffectiveRunContext>> {
        let context = manager.get_status(id)?.effective_context.clone()?;
        let usable = context.usable(id).and_then(|()| {
            check_rung(id, context.tier, self.agent_config.subagent_isolation_max_tier)
        });
        match usable {
            Ok(()) => Some(context),
            Err(reason) => {
                tracing::warn!(subagent_id = %id, %reason, "validation repair runs under the workflow's definition");
                None
            }
        }
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
        self.configure_resume_and_progress(&mut runner, &ids.manager_id)
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
