//! The effective context a workflow's validation repair continues with,
//! captured after placement and runner setup and kept only in this process.
use super::*;
use archon_tools::{isolation::IsolationTier, worktree_manager::WorktreeInfo};

/// The actual runner objects, rather than a recipe for rebuilding confinement.
/// The prototype has no manager, history, writer or progress references: keeping
/// it in a manager entry cannot create a manager -> runner -> manager cycle.
#[derive(Clone)]
pub(crate) struct EffectiveRunContext {
    prototype: SubagentRunner,
    parent_cancel: Option<tokio_util::sync::CancellationToken>,
    pub(crate) tier: IsolationTier,
    pub(crate) worktree: Option<WorktreeInfo>,
    pub(crate) host_timeout: archon_tools::host_timeout::HostTimeout,
    /// Why this context can never be continued exactly, when it cannot.
    unrestorable: Option<String>,
    /// Which directory the agent was placed in, whoever made it, so the one
    /// found at that path later is accepted only if it is still that one.
    placement: Option<archon_tools::placement_identity::PlacementIdentity>,
}

impl std::fmt::Debug for EffectiveRunContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never include the runner or registry: they may contain credentials.
        f.debug_struct("EffectiveRunContext")
            .field("tier", &self.tier)
            .finish_non_exhaustive()
    }
}

impl EffectiveRunContext {
    pub(crate) async fn capture(
        runner: &mut SubagentRunner,
        tier: IsolationTier,
        worktree: Option<WorktreeInfo>,
        parent_cancel: Option<tokio_util::sync::CancellationToken>,
    ) -> Self {
        // Freeze the live parent controls too. The original run and every
        // resume use this same config object, including all enforced limits.
        let mut config = (*runner.agent_config).clone();
        config.fast_mode = Arc::new(std::sync::atomic::AtomicBool::new(
            config.fast_mode.load(std::sync::atomic::Ordering::Relaxed),
        ));
        let effort = *config.effort_level.lock().await;
        let model_override = config.model_override.lock().await.clone();
        let permission_mode = config.permission_mode.lock().await.clone();
        let extra_dirs = config.extra_dirs.lock().await.clone();
        config.effort_level = Arc::new(tokio::sync::Mutex::new(effort));
        config.model_override = Arc::new(tokio::sync::Mutex::new(model_override));
        config.permission_mode = Arc::new(tokio::sync::Mutex::new(permission_mode));
        config.extra_dirs = Arc::new(tokio::sync::Mutex::new(extra_dirs));
        runner.agent_config = Arc::new(config);
        if runner.effort.is_none() {
            runner.effort = Some(runner.agent_config.effort_level.lock().await.to_string());
        }
        let mut prototype = runner.clone();
        prototype.tool_context.cancel_parent = None;
        prototype.subagent_manager = None;
        prototype.runner_agent_id = None;
        prototype.progress = None;
        prototype.shutdown_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        prototype.initial_messages = None;
        prototype.completed_history = None;
        prototype.transcript_store = None;
        prototype.transcript_agent_id = None;
        // The continued run must not share a backend that can change: it gets
        // one frozen as the sandbox is now, or none at all.
        let mut unrestorable = None;
        if let Some(sandbox) = prototype.tool_context.sandbox.clone() {
            match sandbox.snapshot() {
                archon_permissions::SandboxSnapshot::Fixed => {}
                archon_permissions::SandboxSnapshot::Frozen(frozen) => {
                    prototype.tool_context.sandbox = Some(frozen);
                }
                archon_permissions::SandboxSnapshot::Unavailable => {
                    prototype.tool_context.sandbox = None;
                    unrestorable =
                        Some("its sandbox can change after spawn and cannot be frozen".to_string());
                }
            }
        }
        let mut config = (*prototype.agent_config).clone();
        config.sandbox = prototype.tool_context.sandbox.clone();
        prototype.agent_config = Arc::new(config);
        let placement = archon_tools::placement_identity::PlacementIdentity::of(
            &prototype.tool_context.working_dir,
        )
        .map_err(|why| unrestorable = Some(why))
        .ok();
        Self {
            placement,
            prototype,
            parent_cancel,
            unrestorable,
            tier,
            worktree,
            host_timeout: archon_tools::host_timeout::current().unwrap_or(
                archon_tools::host_timeout::HostTimeout::Finite(runner.timeout_secs),
            ),
        }
    }

    pub(crate) fn activity_agent_type(&self) -> &str {
        self.prototype
            .activity_actor_name
            .as_deref()
            .unwrap_or("general-purpose")
    }

    /// Why this context can no longer run exactly as it did, or `Ok`.
    pub(crate) fn usable(&self, agent_id: &str) -> Result<(), String> {
        if let Some(why) = &self.unrestorable {
            return Err(format!(
                "cannot continue agent '{agent_id}': {why}; start a new agent"
            ));
        }
        if !self.prototype.tool_context.working_dir.is_dir() {
            return Err(format!(
                "cannot continue agent '{agent_id}': its original working directory is unavailable; start a new agent"
            ));
        }
        if let Some(placement) = &self.placement
            && let Err(why) = placement.check(&self.prototype.tool_context.working_dir)
        {
            return Err(format!(
                "cannot continue agent '{agent_id}': its working directory is gone: {why}; start a new agent"
            ));
        }
        if self
            .parent_cancel
            .as_ref()
            .is_some_and(|parent| parent.is_cancelled())
        {
            return Err(format!(
                "cannot continue agent '{agent_id}': its original execution scope was cancelled; start a new agent"
            ));
        }
        Ok(())
    }

    /// `caller` is the supervision of the run that asked for this repair. It
    /// narrows the continued run as the original scope does: either stops it.
    pub(crate) fn runner(
        &self,
        agent_id: &str,
        cancel: &tokio_util::sync::CancellationToken,
        caller: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<SubagentRunner, String> {
        self.usable(agent_id)?;
        let mut runner = self.prototype.clone();
        // Cancellation, progress and shutdown are per execution. The original
        // supervision token still narrows the new run; it is never bypassed.
        let tool_cancel = cancel.child_token();
        if let Some(parent) = self.parent_cancel.clone() {
            link_cancellation(parent, &tool_cancel);
        }
        if let Some(caller) = caller.cloned() {
            link_cancellation(caller, &tool_cancel);
        }
        runner.tool_context.cancel_parent = Some(tool_cancel);
        Ok(runner)
    }
}

/// Cancel `tool_cancel` when `scope` is cancelled, at once if it already is.
fn link_cancellation(
    scope: tokio_util::sync::CancellationToken,
    tool_cancel: &tokio_util::sync::CancellationToken,
) {
    if scope.is_cancelled() {
        tool_cancel.cancel();
        return;
    }
    let linked = tool_cancel.clone();
    archon_observability::spawn_named("subagent-repair-cancel-link", async move {
        tokio::select! {
            _ = scope.cancelled() => linked.cancel(),
            _ = linked.cancelled() => {},
        }
    });
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;

/// End each execution's tool scope even when its runner future is abandoned.
pub(crate) struct ToolCancellation(pub(crate) tokio_util::sync::CancellationToken);
impl Drop for ToolCancellation {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl SubagentRunner {
    pub(crate) fn tool_cancellation(&self) -> Option<ToolCancellation> {
        self.tool_context
            .cancel_parent
            .clone()
            .map(ToolCancellation)
    }
}
