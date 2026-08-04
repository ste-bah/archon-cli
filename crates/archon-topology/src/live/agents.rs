//! Invariant 1 — the agent cap.
//!
//! # What was missing
//!
//! Workflows already enforce this at fan-out admission
//! (`archon-workflow/src/fanout.rs`). Teams and plain sessions did not.
//! `AgentPool` (`archon-core/src/orchestrator/pool.rs`) caps *concurrent*
//! agents and releases the slot on completion, so a team could spawn any number
//! of agents over its lifetime provided it never held too many at once —
//! finding O2. Worse, `run_dag_waves` never touched `AgentPool` at all while
//! `run_parallel` did, so `ExecutionMode::Dag` had no concurrency cap either
//! (finding O4).
//!
//! Both of those are defects *inside the orchestrator* and are fixed there:
//! `AgentPool` grew a lifetime total and `run_dag_waves` now uses the pool.
//! This module is the other half — the same ceiling extended to plain sessions,
//! at the one seam every spawn passes through, the tool-run admission callback.
//!
//! # Lifetime, not concurrency
//!
//! [`GraphBudget::max_agents`](crate::ir::GraphBudget::max_agents) is
//! documented as "maximum agents over the graph's whole lifetime — not a
//! concurrency cap", and that is what is counted here.
//! [`SessionState::live_agents`](super::SessionState::live_agents) is tracked
//! too, but it is bookkeeping for the write-conflict check ("is that node still
//! live?"), not a second ceiling. Concurrency remains `AgentPool`'s job.

use super::LiveTopologyConfig;
use super::state::SessionState;
use super::verdict::{Invariant, SpawnIntent, Verdict};

/// Admit a spawn against the lifetime agent budget, accounting for it when
/// admitted.
///
/// Accounting happens only on admission, so a blocked spawn does not consume
/// budget it never used — otherwise one over-cap attempt would poison the
/// remainder of the session.
pub(super) fn admit_spawn(
    state: &mut SessionState,
    config: LiveTopologyConfig,
    intent: &SpawnIntent,
) -> Verdict {
    if !config.agent_cap {
        state.record_spawn(&intent.node_id, intent.parent_id.as_deref());
        return Verdict::Allowed;
    }

    let cap = state.budget().max_agents;
    let spawned = state.agents_spawned();
    if spawned >= cap {
        return Verdict::blocked(
            Invariant::AgentCap,
            format!(
                "agent_cap: spawning '{node}' (agent '{agent}') would be agent {next} of a \
                 lifetime budget of {cap} for this session; {spawned} have already been spawned \
                 and the budget counts every agent ever started, not the number running now. \
                 Do the remaining work in an existing agent, or raise \
                 `[topology] max_agents`.",
                node = intent.node_id,
                agent = intent.agent,
                next = spawned.saturating_add(1),
            ),
        );
    }

    state.record_spawn(&intent.node_id, intent.parent_id.as_deref());
    Verdict::Allowed
}
