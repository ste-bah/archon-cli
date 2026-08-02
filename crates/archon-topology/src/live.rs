//! Milestone 3 — guardrail admission over the **executed prefix**.
//!
//! # The constraint that shapes everything here
//!
//! `ToolRunAdmissionCallback` is
//! `Arc<dyn Fn(ToolRunAdmissionRequest) -> ToolRunAdmission + Send + Sync>` —
//! **synchronous**, on the critical path of every non-`Safe` tool call. A Cozo
//! read here would take a lock; a Cozo write would take the process-wide write
//! lock on every tool call in the process, and the guarded retry budget parks a
//! thread for roughly nineteen seconds in the worst case.
//!
//! So admission is in-memory only. Not "in-memory except for a cheap read" —
//! there is no database in this crate's dependency graph, and there is nothing
//! here that opens a file either. Everything expensive (dominator computation,
//! transitive reachability) happens once, when a graph is declared, and
//! admission is map lookups over the result.
//!
//! # What "executed prefix" means
//!
//! [`SessionState`] is what has actually happened in one session so far: nodes
//! started and finished, gates *passed*, agents spawned, write claims held,
//! loop rounds counted. It is not the declared graph — a declared graph is
//! optional and most turns never declare one. When there is one it is folded in
//! as structure (dependency edges, dominating gates); when there is not, the
//! structure is what the session was observed to do, which for a plain turn is
//! a root node plus whatever it spawned.
//!
//! # The three invariants
//!
//! All three are safety-shaped. Each is individually disableable
//! ([`LiveTopologyConfig`]) and each defaults to on. A session with no
//! registered state admits everything: **the feature never fails closed on a
//! bookkeeping gap.** That rule is why several checks below decline to conclude
//! rather than blocking on missing information, and why the one place it does
//! not apply — a malformed glob, which conflicts with everything — is called
//! out where it happens.
//!
//! 1. [`agents`] — agent cap. A lifetime total, not a concurrency cap.
//! 2. [`writes`] — single writer per artifact.
//! 3. [`gates`] — ungated irreversible action.
//!
//! Loop rounds are recorded ([`SessionState::round`]) but not enforced: the
//! round boundary is not well-defined for an undeclared turn.
//!
//! # Deviations from the design sketch
//!
//! Recorded here rather than in a changelog because each one is a place the
//! specification could not have compiled.
//!
//! * The sketch's methods (`on_spawn(&self, node, agent)`, `on_write_intent`,
//!   `on_tool`) take no session id, yet the state they describe is
//!   `DashMap<SessionId, GraphState>`. The key has to be an argument.
//! * `on_tool(&self, req: &ToolRunAdmissionRequest)` would put `archon-tools`
//!   — tokio, reqwest, the whole tool registry — into this crate's dependency
//!   graph, which is exactly what the dependency budget exists to prevent. The
//!   intent types here ([`ToolIntent`], [`SpawnIntent`], [`WriteIntent`]) are
//!   crate-local; the binary translates.
//! * `Admission` is named as though it exists. It does not; [`Verdict`] is
//!   defined here and the binary maps it onto `ToolRunAdmission`.
//! * `NodeId` is likewise named as a type. Node ids are `String` in the IR.

mod agents;
mod gates;
mod state;
mod verdict;
mod writes;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use dashmap::DashMap;

use crate::ir::{GraphBudget, TaskGraph};

pub use state::{SessionState, SessionStructure};
pub use verdict::{Invariant, SpawnIntent, ToolIntent, Verdict, WriteIntent};

/// How many sessions may be tracked at once.
///
/// A bound rather than unbounded growth because this map lives for the life of
/// the process and a leaked [`LiveTopology::end_session`] would otherwise
/// accumulate forever. Overflow drops the *new* session rather than evicting an
/// old one: an untracked session admits everything, which is the fail-open
/// direction, whereas evicting a live session would discard write claims that
/// concurrent nodes are still relying on.
pub const MAX_TRACKED_SESSIONS: usize = 256;

/// Which sessions the ungated-irreversible invariant applies to.
///
/// **This enum exists because of a defect in the design.** The specification
/// says an irreversible action "not dominated by a passed gate is blocked",
/// "evaluated against the executed prefix, so it works even when no graph was
/// ever declared". Evaluated literally, with the invariant on by default, that
/// blocks *every* irreversible action in *every* plain session — because a
/// plain session has no gate construct at all, so no gate can ever have passed,
/// so nothing is ever dominated by one. Every `git push` in the tool, blocked,
/// by default. The design's own stated bar for these invariants is "near-zero
/// false positives"; that is a hundred percent.
///
/// The rule that recovers the intent is [`GateEnforcement::WhereDeclared`]: an
/// irreversible action is blocked when the graph *declares gates* and none that
/// dominates it has passed. A graph with no gates never opted into gating, so
/// blocking against it asserts an intent nobody expressed. A graph that does
/// declare a gate and then reaches a deploy without passing it is the actual
/// hazard, and that is caught.
///
/// [`GateEnforcement::Always`] is the literal reading, kept because it is what
/// the design asked for and because a deployment that genuinely wants
/// "irreversible actions require a gate, full stop" can have it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateEnforcement {
    /// Never block on this invariant.
    Off,
    /// Block only when the session's structure declares at least one gate.
    #[default]
    WhereDeclared,
    /// Block whenever no passed gate dominates the action, gates declared or
    /// not. Blocks every irreversible action in an undeclared session.
    Always,
}

/// Which invariants enforce. Every one defaults to on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveTopologyConfig {
    /// Block a spawn that would exceed the graph's lifetime agent budget.
    pub agent_cap: bool,
    /// Block a write to a path a concurrently live, unrelated node claims.
    pub single_writer: bool,
    /// Block an irreversible action no passed gate dominates.
    pub ungated_irreversible: GateEnforcement,
    /// Lifetime agent ceiling for a session that declares no graph.
    ///
    /// A declared graph carries its own
    /// [`GraphBudget::max_agents`](crate::ir::GraphBudget::max_agents) and that
    /// wins: an authored budget is a stronger statement than a global default.
    /// This is the number for an ordinary turn, which declares nothing — and
    /// without it the agent cap would silently enforce the IR default instead
    /// of what the operator configured.
    pub max_agents: u32,
}

impl Default for LiveTopologyConfig {
    fn default() -> Self {
        Self {
            agent_cap: true,
            single_writer: true,
            ungated_irreversible: GateEnforcement::default(),
            max_agents: GraphBudget::default().max_agents,
        }
    }
}

impl LiveTopologyConfig {
    /// Every invariant off. For a caller that wants tracking without
    /// enforcement.
    #[must_use]
    pub fn all_disabled() -> Self {
        Self {
            agent_cap: false,
            single_writer: false,
            ungated_irreversible: GateEnforcement::Off,
            ..Self::default()
        }
    }

    /// Every invariant on, with the literal reading of invariant 3.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            agent_cap: true,
            single_writer: true,
            ungated_irreversible: GateEnforcement::Always,
            ..Self::default()
        }
    }
}

/// Per-session live topology state, shared across threads.
///
/// Cheap to clone — the `DashMap` is behind an `Arc`. One instance per process
/// is the expected shape; sessions come and go inside it.
#[derive(Debug, Clone, Default)]
pub struct LiveTopology {
    sessions: Arc<DashMap<String, SessionState>>,
    config: LiveTopologyConfig,
}

impl LiveTopology {
    /// A tracker enforcing `config`.
    #[must_use]
    pub fn new(config: LiveTopologyConfig) -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            config,
        }
    }

    /// The configuration in force.
    #[must_use]
    pub fn config(&self) -> LiveTopologyConfig {
        self.config
    }

    /// Whether `session_id` is tracked. An untracked session admits everything.
    #[must_use]
    pub fn tracks(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// Number of tracked sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Start tracking `session_id`.
    ///
    /// Idempotent: re-registering an already-tracked session leaves its
    /// executed prefix alone, because losing write claims mid-session would
    /// un-block a real conflict. Returns `false` when the session was not taken
    /// on — already present, or the bound is reached.
    pub fn begin_session(&self, session_id: &str) -> bool {
        if self.sessions.contains_key(session_id) {
            return false;
        }
        if self.sessions.len() >= MAX_TRACKED_SESSIONS {
            return false;
        }
        let mut state = SessionState::default();
        // Seed the configured ceiling. A later `declare_graph` overwrites it
        // with the authored budget, which is the stronger statement.
        state.set_budget(GraphBudget {
            max_agents: self.config.max_agents,
            ..GraphBudget::default()
        });
        self.sessions.insert(session_id.to_string(), state);
        true
    }

    /// Stop tracking `session_id` and drop its state. Idempotent.
    pub fn end_session(&self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    /// Attach a declared graph to a tracked session.
    ///
    /// This is where the expensive work happens — dominator computation and
    /// transitive reachability, both once — so that admission stays map
    /// lookups. A graph that fails structural validation (cycle, duplicate id,
    /// unknown dependency) is *ignored*: it is not admission's job to report a
    /// malformed graph, and refusing to track one would fail closed on it.
    ///
    /// A no-op for an untracked session.
    pub fn declare_graph(&self, session_id: &str, graph: &TaskGraph) {
        let Some(structure) = SessionStructure::from_graph(graph) else {
            return;
        };
        if let Some(mut state) = self.sessions.get_mut(session_id) {
            state.adopt_structure(structure);
        }
    }

    /// Run `f` against a tracked session's state.
    ///
    /// Returns [`Verdict::Allowed`] for an untracked session — the one rule
    /// that overrides every invariant.
    fn with_session<F>(&self, session_id: &str, f: F) -> Verdict
    where
        F: FnOnce(&mut SessionState) -> Verdict,
    {
        match self.sessions.get_mut(session_id) {
            Some(mut state) => f(&mut state),
            None => Verdict::Allowed,
        }
    }

    /// Run `f` against a tracked session's state for its side effects only.
    fn record<F>(&self, session_id: &str, f: F)
    where
        F: FnOnce(&mut SessionState),
    {
        if let Some(mut state) = self.sessions.get_mut(session_id) {
            f(&mut state);
        }
    }

    /// Admit a subagent spawn, and account for it when admitted.
    ///
    /// Invariant 1. See [`agents`].
    pub fn on_spawn(&self, session_id: &str, intent: &SpawnIntent) -> Verdict {
        let config = self.config;
        self.with_session(session_id, |state| {
            agents::admit_spawn(state, config, intent)
        })
    }

    /// Release one live agent slot. The lifetime total is not decremented —
    /// that is the whole point of a lifetime cap (finding O2).
    pub fn on_agent_finished(&self, session_id: &str, node_id: &str) {
        self.record(session_id, |state| state.finish_agent(node_id));
    }

    /// Admit a write, and claim the paths when admitted.
    ///
    /// Invariant 2. See [`writes`].
    pub fn on_write_intent(&self, session_id: &str, intent: &WriteIntent) -> Verdict {
        let config = self.config;
        self.with_session(session_id, |state| {
            writes::admit_write(state, config, intent)
        })
    }

    /// Release every write claim held by `node_id`.
    pub fn on_writes_released(&self, session_id: &str, node_id: &str) {
        self.record(session_id, |state| state.release_claims(node_id));
    }

    /// Admit a tool call.
    ///
    /// Runs invariant 3, plus invariant 2 for any path the call declares it
    /// writes and invariant 1 when the call is a spawn. Order is cheapest and
    /// most specific first, so the reason a caller sees names the most
    /// actionable problem.
    pub fn on_tool(&self, session_id: &str, intent: &ToolIntent) -> Verdict {
        let config = self.config;
        self.with_session(session_id, |state| {
            state.start_node(&intent.node_id);
            if let Some(spawn) = intent.spawn.as_ref() {
                let verdict = agents::admit_spawn(state, config, spawn);
                if verdict.is_blocked() {
                    return verdict;
                }
            }
            if !intent.writes.is_empty() {
                let write = WriteIntent::exclusive(intent.node_id.clone(), intent.writes.clone());
                let verdict = writes::admit_write(state, config, &write);
                if verdict.is_blocked() {
                    return verdict;
                }
            }
            gates::admit_irreversible(state, config, intent)
        })
    }

    /// Record that a gate node in the executed prefix has been **passed**.
    ///
    /// The only way a gate becomes passed. Milestone 1's
    /// [`TaskGraph::ungated_irreversible`] evaluates gate *presence*, because a
    /// static graph has no notion of one having been passed; this is the
    /// narrowing the design asks for.
    ///
    /// **`StageKind::Checkpoint` has no execution semantics anywhere in the
    /// tree — nothing marks a checkpoint passed — so under this narrowing a
    /// checkpoint never gates.** That is intended and it is the fail-safe
    /// direction. See the tripwire in `lower_workflow.rs`: specs persisted
    /// before the W6 deletion deserialize a legacy `condition` stage to
    /// `Checkpoint`, and a condition stage never had an evaluator, so treating
    /// its mere presence as gating would let an unevaluated condition authorise
    /// a deploy. Do not "fix" this by counting presence as passed.
    pub fn on_gate_passed(&self, session_id: &str, node_id: &str) {
        self.record(session_id, |state| state.pass_gate(node_id));
    }

    /// Record a node starting.
    pub fn on_node_started(&self, session_id: &str, node_id: &str) {
        self.record(session_id, |state| state.start_node(node_id));
    }

    /// Record a node finishing: releases its live agent slot and its write
    /// claims.
    pub fn on_node_finished(&self, session_id: &str, node_id: &str) {
        self.record(session_id, |state| state.finish_node(node_id));
    }

    /// Record a loop round for `node_id`. Counted, not enforced — the round
    /// boundary is not well-defined for an undeclared turn.
    pub fn on_round(&self, session_id: &str, node_id: &str) {
        self.record(session_id, |state| state.record_round(node_id));
    }

    /// Read a tracked session's state. Test and diagnostic use.
    #[must_use]
    pub fn snapshot(&self, session_id: &str) -> Option<SessionState> {
        self.sessions.get(session_id).map(|state| state.clone())
    }
}
