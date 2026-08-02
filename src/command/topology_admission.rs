//! Milestone 3 — the wiring between the tool-run seam and
//! [`archon_topology::live`].
//!
//! # The hard constraint
//!
//! `ToolRunAdmissionCallback` is
//! `Arc<dyn Fn(ToolRunAdmissionRequest) -> ToolRunAdmission + Send + Sync>` —
//! synchronous, on the critical path of every non-`Safe` tool call.
//! **Admission performs no database access of any kind, not even a read.** A
//! Cozo read takes a lock; a Cozo write takes the process-wide write lock on
//! every tool call in the process, and the guarded retry budget parks a thread
//! for roughly nineteen seconds in the worst case.
//!
//! Everything reachable from [`admit`] is a `DashMap` lookup and some string
//! work. `archon-topology` declares no `cozo` dependency, so that is a property
//! of the build graph, and
//! `topology_admission::tests::hot_path` proves it behaviourally by arming
//! every guarded Cozo operation on the thread to panic and then driving a
//! session through admission.
//!
//! # Where this sits relative to the world-model guardrail
//!
//! Both are installed on the one `ToolRunAdmissionCallback`, which has no
//! registry behind it, so composition happens by hand in
//! [`crate::command::world_model::configure_tool_run_context`] and
//! `src/session/world_model_callbacks.rs`. Topology runs **first**: it is
//! in-memory and cheap, while the world-model guardrail persists a candidate
//! trace and a revision record before it answers. Blocking early skips those
//! writes.
//!
//! # Attribution, and its limit
//!
//! `ToolContext::tool_run_parent_action_id` is copied verbatim into every
//! subagent's context, so a tool call made deep inside a spawned agent reports
//! the same parent action id as one made by the top-level agent. Milestone 2
//! recorded this honestly and milestone 3 inherits it: a tool call attributes
//! to the turn root unless it *is* a spawn, which the tool name reveals. The
//! consequence for invariant 2 is that two subagents writing the same file both
//! look like the root writing it twice, which is a self-conflict and therefore
//! admitted — under-reporting, never over-reporting. Fixing it needs a node
//! identifier on `ToolContext`, which is new plumbing and out of scope here.

mod session;
mod translate;

#[cfg(test)]
mod tests;

use archon_tools::tool::{ToolRunAdmission, ToolRunAdmissionRequest, ToolRunAttemptOutcome};
use archon_topology::live::{LiveTopology, Verdict};

pub(crate) use session::{
    active, begin_session, declare_graph, end_session, install, on_gate_passed, on_node_finished,
    on_node_started, uninstall,
};

/// Admit one tool attempt.
///
/// Safe to call unconditionally. Returns [`ToolRunAdmission::Allowed`] when no
/// [`LiveTopology`] is installed, when the session is not tracked, or when
/// every invariant is disabled — **the feature never fails closed on a
/// bookkeeping gap.**
pub(crate) fn admit(request: &ToolRunAdmissionRequest) -> ToolRunAdmission {
    let Some(live) = active() else {
        return ToolRunAdmission::Allowed;
    };
    let intent = translate::tool_intent(request);
    verdict_to_admission(live.on_tool(&request.session_id, &intent))
}

/// Release what a finished tool attempt was holding.
///
/// Wired into the composed `ToolRunOutcomeCallback` beside the milestone 2
/// trace tap. A spawn's live-agent slot and a write's path claims are held from
/// admission until the attempt terminates; without this they would leak and
/// every later write would look like a conflict.
///
/// The lifetime agent total is deliberately *not* released — that is the whole
/// point of a lifetime cap, and the distinction `AgentPool` failed to make
/// (finding O2).
pub(crate) fn on_tool_run_outcome(outcome: &ToolRunAttemptOutcome) {
    let Some(live) = active() else {
        return;
    };
    if translate::is_spawn(&outcome.tool_name) {
        let node_id = translate::node_id(outcome.tool_use_id.as_str(), outcome.attempt);
        live.on_node_finished(&outcome.session_id, &node_id);
    }
    // Only for a call that actually claimed something. Releasing the turn
    // root's claims after every attempt — a `Read` included — would drop a
    // claim a concurrently running write still holds. That under-reports rather
    // than over-reports, but there is no reason to accept even that.
    if !translate::declared_writes(&outcome.tool_name, &outcome.input).is_empty() {
        live.on_writes_released(&outcome.session_id, translate::turn_root());
    }
}

/// Map a topology verdict onto the tool-run admission answer.
///
/// The reason travels verbatim. `execute_tool_attempt` surfaces it as
/// `"ToolRun blocked: {reason}"` in the tool result, which the model reads —
/// which is why the reasons name the conflicting node and the invariant rather
/// than saying "blocked by policy". A reason the model cannot act on produces a
/// retry loop.
fn verdict_to_admission(verdict: Verdict) -> ToolRunAdmission {
    match verdict {
        Verdict::Allowed => ToolRunAdmission::Allowed,
        Verdict::Blocked { invariant, reason } => {
            tracing::debug!(invariant = invariant.id(), %reason, "topology admission blocked");
            ToolRunAdmission::Blocked { reason }
        }
    }
}

/// Resolve the enforcement configuration from `[topology]`.
fn live_config(
    config: &archon_core::config::TopologyConfig,
) -> archon_topology::live::LiveTopologyConfig {
    use archon_core::config::GateEnforcementConfig;
    use archon_topology::live::GateEnforcement;

    archon_topology::live::LiveTopologyConfig {
        agent_cap: config.agent_cap,
        single_writer: config.single_writer,
        ungated_irreversible: match config.ungated_irreversible {
            GateEnforcementConfig::Off => GateEnforcement::Off,
            GateEnforcementConfig::WhereDeclared => GateEnforcement::WhereDeclared,
            GateEnforcementConfig::Always => GateEnforcement::Always,
        },
        max_agents: config.max_agents,
    }
}

/// Build a tracker for `config`, or `None` when admission is switched off
/// wholesale.
fn tracker_for(config: &archon_core::config::TopologyConfig) -> Option<LiveTopology> {
    config
        .admission_enabled
        .then(|| LiveTopology::new(live_config(config)))
}
