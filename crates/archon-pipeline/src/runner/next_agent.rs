//! The [`NextAgent`] instruction and the contract the runner enforces on it.

use anyhow::Result;

use super::{AgentInfo, PipelineSession};

/// Largest number of agents one [`NextAgent::ContinueWave`] may carry.
///
/// Facades that emit waves must cap them with this constant. The runner
/// rejects a larger wave; it never truncates one.
pub const PARALLEL_WAVE_LIMIT: usize = 4;

/// Largest number of consecutive [`NextAgent::Skip`] instructions the runner
/// accepts before it fails the run.
///
/// The count resets on every step that records a result. The bound is far
/// above any real pipeline's agent count; reaching it means the facade is
/// skipping without advancing.
pub const MAX_CONSECUTIVE_SKIPS: usize = 1000;

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
    /// its own state. More than [`MAX_CONSECUTIVE_SKIPS`] skips in a row, with
    /// no recorded result between them, fails the run with an error that
    /// names the last reason and the count.
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

/// The agent keys a `Continue` or `ContinueWave` step must append.
pub(super) struct StepProgress {
    variant: &'static str,
    expected: Vec<String>,
    before: usize,
}

impl StepProgress {
    pub(super) fn start(
        session: &PipelineSession,
        variant: &'static str,
        agents: &[AgentInfo],
    ) -> Self {
        Self {
            variant,
            expected: agents.iter().map(|agent| agent.key.clone()).collect(),
            before: session.agent_results.len(),
        }
    }

    /// Fail unless the step appended exactly its own agents, in order, and
    /// left the prior results in place.
    ///
    /// Without this check a step that records nothing makes the facade emit
    /// the same step again, and the runner loops forever.
    pub(super) fn check(self, session: &PipelineSession) -> Result<()> {
        let after = session.agent_results.len();
        let appended: Option<Vec<&str>> = session
            .agent_results
            .get(self.before..)
            .map(|new| new.iter().map(|(agent, _)| agent.key.as_str()).collect());
        if appended.as_ref().is_some_and(|keys| *keys == self.expected) {
            return Ok(());
        }
        let actual = match appended {
            Some(keys) => format!("appended [{}]", keys.join(", ")),
            None => format!("the {} prior result(s) shrank to {}", self.before, after),
        };
        let message = format!(
            "pipeline runner progress violation: NextAgent::{} expected to append \
             [{}] after {} prior result(s), but {}",
            self.variant,
            self.expected.join(", "),
            self.before,
            actual
        );
        tracing::error!(
            session_id = %session.id,
            error = %message,
            "pipeline.agent.step_without_progress"
        );
        Err(anyhow::anyhow!(message))
    }
}

/// Counts consecutive [`NextAgent::Skip`] instructions.
#[derive(Default)]
pub(super) struct SkipGuard {
    consecutive: usize,
}

impl SkipGuard {
    /// Call after a step that recorded results.
    pub(super) fn reset(&mut self) {
        self.consecutive = 0;
    }

    /// Record one skip; fail once the run exceeds [`MAX_CONSECUTIVE_SKIPS`].
    pub(super) fn record(&mut self, session: &PipelineSession, reason: &str) -> Result<()> {
        self.consecutive += 1;
        if self.consecutive <= MAX_CONSECUTIVE_SKIPS {
            return Ok(());
        }
        let message = format!(
            "pipeline runner progress violation: {} consecutive NextAgent::Skip with no \
             recorded result (limit MAX_CONSECUTIVE_SKIPS = {}); last reason: {}",
            self.consecutive, MAX_CONSECUTIVE_SKIPS, reason
        );
        tracing::error!(
            session_id = %session.id,
            error = %message,
            "pipeline.agent.skip_limit_exceeded"
        );
        Err(anyhow::anyhow!(message))
    }
}
