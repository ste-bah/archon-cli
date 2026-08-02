//! The ambient topology trace: three taps, one append-only jsonl file.
//!
//! # What this is
//!
//! Milestone 2 wants an outcome corpus keyed by the *shape* work ran in. To get
//! one, every turn has to leave a record of what it spawned, what it called,
//! and what it wrote. This module is the recording side. The reading side is
//! [`crate::command::topology_fold`], which is the only thing that ever touches
//! a database.
//!
//! # Why nothing here writes to Cozo
//!
//! The Cozo stores are SQLite-backed behind a process-wide write lock keyed by
//! canonicalized path (`archon-cozo::locking`). A write from here would take
//! that lock on every tool call in the process, and the guarded retry budget
//! parks a thread for roughly 19 seconds in the worst case. So the hot path
//! appends a line to a file and stops. `archon-topology` has no `cozo`
//! dependency, which makes that a property of the build graph rather than a
//! rule to remember.
//!
//! # The three taps
//!
//! All three are pre-existing seams; none required threading a new parameter
//! through a call stack. Each one owns a file beside this one.
//!
//! 1. **`ToolRunOutcomeCallback`** ([`tool_tap`]) — installed at
//!    [`crate::command::world_model::configure_tool_run_context`] and
//!    `src/session/world_model_callbacks.rs`. It now fires for every attempt
//!    rather than only admitted ones; see the C2 note on
//!    `archon_core::tool_run_admission::record_outcome`.
//! 2. **`OrchestratorEvent`** ([`orchestrator_tap`]) — projected from the
//!    receiver loop in `src/command/team.rs`. There is no subscriber registry
//!    on that channel, only a single `mpsc::Sender` threaded through
//!    `Orchestrator::run_team`, so the receiver loop is the seam.
//! 3. **Workflow events** ([`workflow_tap`]) — `WorkflowEvent` values,
//!    projected as they are emitted or replayed from `events.jsonl`.
//!
//! Key names the taps have to guess at, in either a tool input or an event
//! `detail`, are all in [`payload`].
//!
//! # Attribution, and its limit
//!
//! The design document states that the tool-run callback supplies "the parent
//! action / tool-use identifiers needed to attribute a record to a node". It
//! does not. `ToolContext::tool_run_parent_action_id` is copied verbatim into
//! every subagent's context
//! (`archon_core::subagent_executor::run_runner`), so a tool call made deep
//! inside a spawned agent reports exactly the same parent action id as one made
//! by the top-level agent. `session_id` is inherited the same way.
//!
//! So the tool tap alone cannot say *which* node made a call. What it can do is
//! notice the spawn itself — a subagent is launched by a tool call — and emit
//! an `agent_spawned` record for it. Everything else attributes to the turn
//! root. Per-child tool attribution needs a node identifier on `ToolContext`,
//! which is new plumbing and therefore out of scope here. The consequence is
//! recorded honestly in
//! [`archon_topology::reconstruct`]: a reconstructed graph recovers structure,
//! not intent.

mod orchestrator_tap;
mod payload;
mod tool_tap;
mod workflow_tap;

use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use archon_core::orchestrator::events::OrchestratorEvent;
use archon_tools::tool::ToolRunAttemptOutcome;
use archon_topology::ir::TaskGraph;
use archon_topology::trace::{TopologyPaths, TraceKind, TraceRecord, TraceWriter};

pub(crate) use payload::{subagent_type, written_paths};
pub(crate) use workflow_tap::project_workflow_run;

/// One turn's ambient trace.
///
/// Cheap to clone through the `Arc` the registry hands out. Holds no file
/// handle — [`TraceWriter`] reopens per append, which is what lets concurrent
/// workers interleave whole lines with no lock.
#[derive(Debug)]
pub(crate) struct AmbientTrace {
    graph_id: String,
    session_id: String,
    paths: TopologyPaths,
    writer: TraceWriter,
}

impl AmbientTrace {
    /// Open a trace for `graph_id` under `<project_root>/.archon/topology`.
    pub(crate) fn open(
        project_root: &Path,
        graph_id: &str,
        session_id: &str,
    ) -> std::io::Result<Self> {
        let paths = TopologyPaths::for_project(project_root);
        let writer = paths.writer(graph_id)?;
        Ok(Self {
            graph_id: graph_id.to_string(),
            session_id: session_id.to_string(),
            paths,
            writer,
        })
    }

    // Read-only accessors used by the trace and fold test suites to assert on
    // what actually landed on disk. Production code holds the paths it needs.
    #[cfg(test)]
    pub(crate) fn graph_id(&self) -> &str {
        &self.graph_id
    }

    #[cfg(test)]
    pub(crate) fn paths(&self) -> &TopologyPaths {
        &self.paths
    }

    /// Persist a declared graph so the fold reads an authored shape rather than
    /// reconstructing one. Optional by design — a plain turn declares nothing.
    pub(crate) fn declare_graph(&self, graph: &TaskGraph) {
        if let Err(error) = self.paths.write_graph(graph) {
            tracing::debug!(%error, graph_id = %self.graph_id, "topology graph could not be persisted");
        }
        self.record(TraceRecord::new(
            now(),
            &self.graph_id,
            TraceKind::GraphDeclared,
        ));
    }

    /// Append one record.
    ///
    /// **Never propagates an error.** This runs on the hot path of every tool
    /// call; a full disk or a read-only checkout must degrade the corpus, not
    /// the user's turn. Failures log at debug and are dropped.
    pub(crate) fn record(&self, record: TraceRecord) {
        if let Err(error) = self.writer.append(&record) {
            tracing::debug!(%error, graph_id = %self.graph_id, "topology trace append failed");
        }
    }
}

/// Process-wide slot holding the turn's trace, if one is running.
///
/// A global rather than a parameter because the tool-run tap is installed as a
/// bare `fn` pointer
/// (`src/command/world_model.rs::configure_tool_run_context`) with no place to
/// carry state. `RwLock` rather than `OnceLock` because a process runs many
/// turns.
fn slot() -> &'static RwLock<Option<Arc<AmbientTrace>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<AmbientTrace>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Make `trace` the ambient trace for this process.
pub(crate) fn install(trace: Arc<AmbientTrace>) {
    if let Ok(mut slot) = slot().write() {
        *slot = Some(trace);
    }
}

/// Begin an ambient trace and install it. Returns `None` when the trace
/// directory cannot be created — tracing is best-effort and its absence must
/// never fail a turn.
pub(crate) fn begin(
    project_root: &Path,
    graph_id: &str,
    session_id: &str,
) -> Option<Arc<AmbientTrace>> {
    match AmbientTrace::open(project_root, graph_id, session_id) {
        Ok(trace) => {
            let trace = Arc::new(trace);
            install(Arc::clone(&trace));
            Some(trace)
        }
        Err(error) => {
            tracing::debug!(%error, %graph_id, "ambient topology trace could not be opened");
            None
        }
    }
}

/// Stop tracing. Idempotent.
pub(crate) fn end() {
    if let Ok(mut slot) = slot().write() {
        *slot = None;
    }
}

/// The ambient trace, if one is installed.
pub(crate) fn active() -> Option<Arc<AmbientTrace>> {
    slot().read().ok().and_then(|slot| slot.clone())
}

/// Serializes every test that touches process-global topology state.
///
/// Two globals are in play — the ambient trace slot above and
/// `archon_cozo::poison_guarded_scripts` — and a test that installs into either
/// one is visible to every other test in the binary. One lock covering both
/// rather than two is deliberate: the "no hot-path database access" test needs
/// both at once, and two locks would be an ordering hazard for no benefit.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Tool-run tap. Safe to call unconditionally; a no-op when nothing is tracing.
pub(crate) fn on_tool_run_outcome(outcome: &ToolRunAttemptOutcome) {
    if let Some(trace) = active() {
        trace.record_tool_outcome(outcome);
    }
}

/// Orchestrator tap.
pub(crate) fn on_orchestrator_event(event: &OrchestratorEvent) {
    if let Some(trace) = active() {
        trace.record_orchestrator_event(event);
    }
}

/// RFC3339 timestamp. `archon-topology` has no clock dependency, so the caller
/// supplies one; this is that caller.
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests;
