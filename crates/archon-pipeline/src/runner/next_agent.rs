//! The [`NextAgent`] instruction and the contract the runner enforces on it.

use anyhow::Result;

use super::{AgentInfo, PipelineSession};

/// Largest number of agents one [`NextAgent::ContinueWave`] may carry.
///
/// Facades that emit waves must cap them with this constant. The runner
/// rejects a larger wave; it never truncates one.
pub const PARALLEL_WAVE_LIMIT: usize = 4;

/// Instruction from the facade about what to do next.
pub enum NextAgent {
    /// Execute this agent next.
    ///
    /// On success the runner appends exactly one entry to
    /// `session.agent_results`.
    Continue(AgentInfo),
    /// Execute these independent agents as one deterministic bounded wave.
    ///
    /// Contract (checked before any member runs):
    /// - the wave names at least one agent;
    /// - the wave names at most [`PARALLEL_WAVE_LIMIT`] agents;
    /// - every member has `parallelizable == true`.
    ///
    /// The runner runs every member and, on success, appends exactly one entry
    /// per member to `session.agent_results`, in wave order. It never drops a
    /// member. A wave that breaks the contract fails the run with an error
    /// that names the agent keys and the rule broken.
    ContinueWave(Vec<AgentInfo>),
    /// Pipeline is finished.
    Done,
    /// Skip an agent, with a reason string for logging.
    ///
    /// The runner records no result for a skip, so the facade must advance
    /// its own state; the next call to `next_agent` must not repeat the skip
    /// forever.
    Skip(String),
}

fn agent_keys(agents: &[AgentInfo]) -> String {
    agents
        .iter()
        .map(|agent| agent.key.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Reject a wave that breaks the [`NextAgent::ContinueWave`] contract.
pub(super) fn validate_wave(session_id: &str, agents: &[AgentInfo]) -> Result<()> {
    let mut violations = Vec::new();
    if agents.is_empty() {
        violations.push("the wave is empty; a wave must name at least one agent".to_string());
    }
    if agents.len() > PARALLEL_WAVE_LIMIT {
        violations.push(format!(
            "the wave has {} agents [{}]; the limit is PARALLEL_WAVE_LIMIT = {}",
            agents.len(),
            agent_keys(agents),
            PARALLEL_WAVE_LIMIT
        ));
    }
    let serial: Vec<AgentInfo> = agents
        .iter()
        .filter(|agent| !agent.parallelizable)
        .cloned()
        .collect();
    if !serial.is_empty() {
        violations.push(format!(
            "agents [{}] are not parallelizable; every wave member must set parallelizable = true",
            agent_keys(&serial)
        ));
    }
    if violations.is_empty() {
        return Ok(());
    }
    let message = format!(
        "pipeline facade contract violation in NextAgent::ContinueWave: {}",
        violations.join("; ")
    );
    tracing::error!(
        session_id = %session_id,
        wave_size = agents.len(),
        agent_keys = %agent_keys(agents),
        error = %message,
        "pipeline.agent.parallel_wave_rejected"
    );
    Err(anyhow::anyhow!(message))
}

/// Result count a `Continue` or `ContinueWave` step must add to the session.
pub(super) struct StepProgress {
    variant: &'static str,
    keys: String,
    before: usize,
    expected: usize,
}

impl StepProgress {
    pub(super) fn start(
        session: &PipelineSession,
        variant: &'static str,
        agents: &[AgentInfo],
    ) -> Self {
        Self {
            variant,
            keys: agent_keys(agents),
            before: session.agent_results.len(),
            expected: agents.len(),
        }
    }

    /// Fail when a step returned `Ok` but did not add one result per agent.
    ///
    /// Without this check a step that records nothing makes the facade emit
    /// the same step again, and the runner loops forever.
    pub(super) fn check(self, session: &PipelineSession) -> Result<()> {
        let after = session.agent_results.len();
        let added = after.checked_sub(self.before);
        if added == Some(self.expected) {
            return Ok(());
        }
        let message = format!(
            "pipeline runner progress violation: NextAgent::{} [{}] completed but \
             agent_results went from {} to {} (expected {} new result(s))",
            self.variant, self.keys, self.before, after, self.expected
        );
        tracing::error!(
            session_id = %session.id,
            error = %message,
            "pipeline.agent.step_without_progress"
        );
        Err(anyhow::anyhow!(message))
    }
}
