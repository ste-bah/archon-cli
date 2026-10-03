//! A workflow's validation repair continues its call's stored effective
//! context exactly, or is refused (#241). Nothing is rebuilt for it.
use super::*;
use crate::subagent::runner::EffectiveRunContext;
use archon_tools::isolation::IsolationTier;

impl AgentSubagentExecutor {
    /// The stored context a repair (`continuing`) runs with; `None` for a
    /// call that is not a repair. Checked under the manager lock that then
    /// registers the run, so the context is the one of the occupancy that
    /// just finished.
    ///
    /// A repair whose context is gone (another process started the agent, or
    /// it was collected), or can no longer run exactly, is refused. The
    /// workflow handles a refused repair as a failed one.
    pub(super) fn resume_context(
        &self,
        manager: &SubagentManager,
        id: &str,
        continuing: bool,
    ) -> Result<Option<Arc<EffectiveRunContext>>, ExecutorError> {
        if !continuing {
            return Ok(None);
        }
        let context = manager
            .get_status(id)
            .and_then(|info| info.effective_context.clone())
            .ok_or_else(|| {
                ExecutorError::Internal(crate::agents::transcript::resume::unknown_context(id))
            })?;
        context
            .usable(id)
            .and_then(|()| {
                check_rung(
                    id,
                    context.tier,
                    self.agent_config.subagent_isolation_max_tier,
                )
            })
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
        self.configure_resume_and_progress(&mut runner, &ids.manager_id)
            .await;
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
            "cannot continue agent '{id}': it ran on the isolation rung '{}', but subagent.isolation_max_tier now permits only '{}'; raise the cap or start a new agent",
            stored.as_str(),
            cap.as_str()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "run_resume_tests.rs"]
mod tests;
