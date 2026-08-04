//! The process-wide [`LiveTopology`] slot and the session lifecycle around it.
//!
//! A global rather than a parameter for the same reason the milestone 2 trace
//! slot is one: the tool-run admission callback is installed as a closure over
//! a config with no place to carry per-session state, and the callback is
//! invoked from inside `archon-core` with only a
//! `ToolRunAdmissionRequest` in hand. `RwLock` rather than `OnceLock` because a
//! process runs many sessions.

use std::sync::{Arc, OnceLock, RwLock};

use archon_topology::ir::TaskGraph;
use archon_topology::live::LiveTopology;

fn slot() -> &'static RwLock<Option<Arc<LiveTopology>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<LiveTopology>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// The installed tracker, if any. `None` ⇒ everything is admitted.
pub(crate) fn active() -> Option<Arc<LiveTopology>> {
    slot().read().ok().and_then(|slot| slot.clone())
}

/// Install a tracker built from `[topology]`, and begin tracking `session_id`.
///
/// A no-op returning `None` when `[topology] admission_enabled = false`.
/// Idempotent per session: re-installing over an existing tracker keeps that
/// tracker, because replacing it mid-session would discard write claims live
/// nodes still rely on.
pub(crate) fn install(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
) -> Option<Arc<LiveTopology>> {
    if let Some(existing) = active() {
        existing.begin_session(session_id);
        return Some(existing);
    }
    let live = Arc::new(super::tracker_for(&config.topology)?);
    live.begin_session(session_id);
    if let Ok(mut slot) = slot().write() {
        *slot = Some(Arc::clone(&live));
    }
    Some(live)
}

/// Drop the installed tracker. Idempotent.
pub(crate) fn uninstall() {
    if let Ok(mut slot) = slot().write() {
        *slot = None;
    }
}

/// Begin tracking one more session against the installed tracker.
pub(crate) fn begin_session(session_id: &str) {
    if let Some(live) = active() {
        live.begin_session(session_id);
    }
}

/// Stop tracking `session_id` and drop its executed prefix. Idempotent.
pub(crate) fn end_session(session_id: &str) {
    if let Some(live) = active() {
        live.end_session(session_id);
    }
}

/// Drop everything a cancelled turn was holding, and keep tracking the session.
///
/// Claims are held from admission until [`on_tool_run_outcome`] releases them.
/// A cancelled turn never reaches that callback, so a spawn's live-agent slot
/// and a write's path claims leak — and `admit` then refuses every later write
/// in the session as a conflict with a claim nothing still holds. The symptom
/// is that cancelling one command breaks the session permanently, which is far
/// worse than the guardrail simply being absent.
///
/// End-then-begin rather than releasing individual nodes: after a cancel the
/// executed prefix describes work that did not finish, so there is nothing in
/// it worth preserving, and enumerating what was in flight from the cancel path
/// would be guesswork. Tracking continues, so the guardrail stays live for the
/// rest of the session.
///
/// [`on_tool_run_outcome`]: super::on_tool_run_outcome
pub(crate) fn reset_session(session_id: &str) {
    if let Some(live) = active() {
        live.end_session(session_id);
        live.begin_session(session_id);
    }
}

/// Attach a declared graph to a tracked session.
///
/// Called from the milestone 2 taps, which already lower a team decomposition
/// and a workflow spec into the IR — so admission gets the authored shape for
/// free rather than needing its own lowering.
pub(crate) fn declare_graph(session_id: &str, graph: &TaskGraph) {
    if let Some(live) = active() {
        live.declare_graph(session_id, graph);
    }
}

/// Record a node starting in the executed prefix.
pub(crate) fn on_node_started(session_id: &str, node_id: &str) {
    if let Some(live) = active() {
        live.on_node_started(session_id, node_id);
    }
}

/// Record a node finishing: releases its live agent slot and write claims.
pub(crate) fn on_node_finished(session_id: &str, node_id: &str) {
    if let Some(live) = active() {
        live.on_node_finished(session_id, node_id);
    }
}

/// Record that a gate node has been **passed**.
///
/// The only producer wired today is a workflow `HumanGate` stage completing
/// (`topology_trace::workflow_tap`). There is deliberately no producer for a
/// plain turn: nothing in an ordinary session marks a gate passed, which is
/// exactly why `GateEnforcement::WhereDeclared` is the default. See the
/// discussion on `archon_topology::live::GateEnforcement`.
pub(crate) fn on_gate_passed(session_id: &str, node_id: &str) {
    if let Some(live) = active() {
        live.on_gate_passed(session_id, node_id);
    }
}
