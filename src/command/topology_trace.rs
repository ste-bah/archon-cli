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
//! through a call stack.
//!
//! 1. **`ToolRunOutcomeCallback`** — installed at
//!    [`crate::command::world_model::configure_tool_run_context`] and
//!    `src/session/world_model_callbacks.rs`. It now fires for every attempt
//!    rather than only admitted ones; see the C2 note on
//!    `archon_core::tool_run_admission::record_outcome`.
//! 2. **`OrchestratorEvent`** — projected from the receiver loop in
//!    `src/command/team.rs`. There is no subscriber registry on that channel,
//!    only a single `mpsc::Sender` threaded through `Orchestrator::run_team`,
//!    so the receiver loop is the seam.
//! 3. **Workflow events** — `WorkflowEvent` values, projected as they are
//!    emitted or replayed from `events.jsonl`.
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

use std::path::Path;
use std::sync::{Arc, OnceLock, RwLock};

use archon_core::orchestrator::events::OrchestratorEvent;
use archon_tools::tool::{PermissionLevel, ToolRunAttemptOutcome};
use archon_topology::ir::{GraphOrigin, PermissionClass, TaskGraph, WriteTarget};
use archon_topology::reconstruct::ROOT_NODE_ID;
use archon_topology::trace::{TopologyPaths, TraceKind, TraceRecord, TraceWriter};

/// Tools whose invocation launches a subagent. Seeing one of these in the tool
/// tap is how a spawn becomes a node without any new plumbing.
const SUBAGENT_TOOLS: &[&str] = &["Agent", "Task", "TaskCreate"];

/// Tool input keys that name a file the tool writes.
const WRITE_PATH_KEYS: &[&str] = &["file_path", "path", "notebook_path", "target_file"];

/// Tools that write files. Restricting extraction to these avoids recording a
/// `Read`'s `file_path` as a write.
const WRITING_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "ApplyPatch", "NotebookEdit"];

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

    /// Project a tool attempt outcome into trace records.
    ///
    /// Emits a `tool_attempt` always, plus an `agent_spawned` when the tool
    /// launches a subagent and a `file_written` when it names a file it wrote.
    /// Tool *input* is never recorded verbatim — only the extracted paths and
    /// the tool name — so the trace cannot become a secret sink.
    pub(crate) fn record_tool_outcome(&self, outcome: &ToolRunAttemptOutcome) {
        let ts = now();
        let node_id = ROOT_NODE_ID;

        let mut attempt = TraceRecord::new(&ts, &self.graph_id, TraceKind::ToolAttempt)
            .with_node(node_id)
            .with_tool(&outcome.tool_name)
            .with_permission(permission_class(outcome.permission_level))
            .with_outcome(outcome.blocked, outcome.is_error)
            .with_attempt(outcome.attempt);
        let writes = written_paths(&outcome.tool_name, &outcome.input);
        if !writes.is_empty() {
            attempt = attempt.with_writes(writes.clone());
        }
        self.record(attempt);

        if SUBAGENT_TOOLS.contains(&outcome.tool_name.as_str()) && !outcome.blocked {
            // The tool-use id is the only per-invocation identifier available
            // here, so it names the spawned node. It is stable within a turn
            // and meaningless across turns, which is exactly the lifetime a
            // node id needs.
            let child = spawn_node_id(&outcome.tool_use_id, outcome.attempt);
            let mut spawned = TraceRecord::new(&ts, &self.graph_id, TraceKind::AgentSpawned)
                .with_node(child)
                .with_parent(node_id)
                .with_outcome(outcome.blocked, outcome.is_error);
            if let Some(agent) = subagent_type(&outcome.input) {
                spawned = spawned.with_agent(agent);
            }
            self.record(spawned);
        }

        if !writes.is_empty() {
            self.record(
                TraceRecord::new(&ts, &self.graph_id, TraceKind::FileWritten)
                    .with_node(node_id)
                    .with_writes(writes)
                    .with_outcome(outcome.blocked, outcome.is_error),
            );
        }
    }

    /// Project an orchestrator event into trace records.
    ///
    /// Only the four variants the design names are projected. The others
    /// (`AgentProgress`, `TeamFailed`) are never emitted anywhere in the tree,
    /// and `AgentComplete` / `AgentFailed` are handled because they carry the
    /// per-node terminal outcome the corpus needs.
    pub(crate) fn record_orchestrator_event(&self, event: &OrchestratorEvent) {
        let ts = now();
        match event {
            OrchestratorEvent::TaskDecomposed { subtasks } => {
                // The decomposition *is* the declared graph. Lowering it here
                // means a team run folds against an authored shape rather than
                // a reconstruction, which is strictly better information.
                let graph =
                    archon_core::orchestrator::topology::lower_subtasks(subtasks, &self.session_id);
                let graph = TaskGraph {
                    id: self.graph_id.clone(),
                    origin: GraphOrigin::Team {
                        session_id: self.session_id.clone(),
                    },
                    ..graph
                };
                self.declare_graph(&graph);
            }
            OrchestratorEvent::AgentSpawned {
                agent_id,
                agent_type,
                subtask_id,
            } => {
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::AgentSpawned)
                        .with_node(subtask_id)
                        .with_agent(agent_type)
                        .with_detail(agent_id),
                );
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::NodeStarted)
                        .with_node(subtask_id),
                );
            }
            OrchestratorEvent::AgentComplete { subtask_id, .. } => {
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::NodeFinished)
                        .with_node(subtask_id),
                );
            }
            OrchestratorEvent::AgentFailed {
                subtask_id,
                will_retry,
                ..
            } => {
                let kind = if *will_retry {
                    TraceKind::Retry
                } else {
                    TraceKind::NodeFinished
                };
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, kind)
                        .with_node(subtask_id)
                        .with_outcome(false, true),
                );
            }
            OrchestratorEvent::TeamComplete { .. } => {
                self.record(TraceRecord::new(
                    &ts,
                    &self.graph_id,
                    TraceKind::NodeFinished,
                ));
            }
            OrchestratorEvent::TeamCancelled => {
                self.record(
                    TraceRecord::new(&ts, &self.graph_id, TraceKind::NodeFinished)
                        .with_outcome(false, true)
                        .with_detail("team cancelled"),
                );
            }
            // Declared but never emitted anywhere in the tree.
            OrchestratorEvent::AgentProgress { .. } | OrchestratorEvent::TeamFailed { .. } => {}
        }
    }
}

/// Map one workflow event onto a trace record, or `None` when it carries no
/// node-shaped meaning.
///
/// Stage identifiers live in the event's `detail` payload rather than in a
/// typed field, so this reads `detail.stage` / `detail.stage_id` and attributes
/// to the turn root when neither is present.
///
/// Every other kind — lifecycle noise, and the eleven write-coordination kinds
/// — is skipped. Skipping is deliberate rather than lossy: the trace format
/// grows by adding kinds, not by guessing at what an unmapped one means.
fn workflow_trace_record(
    graph_id: &str,
    event: &archon_workflow::WorkflowEvent,
) -> Option<TraceRecord> {
    use archon_workflow::WorkflowEventKind as Kind;

    let kind = match event.kind {
        Kind::StageStarted => TraceKind::NodeStarted,
        Kind::StageCompleted | Kind::StageSkipped => TraceKind::NodeFinished,
        Kind::StageFailed | Kind::StageStalled => TraceKind::Retry,
        Kind::ForcedAccepted => TraceKind::GatePassed,
        Kind::Completed | Kind::Cancelled => TraceKind::NodeFinished,
        _ => return None,
    };

    let node = workflow_stage_id(&event.detail).unwrap_or_else(|| ROOT_NODE_ID.to_string());
    let mut record = TraceRecord::new(event.ts.to_rfc3339(), graph_id, kind).with_node(node);
    if matches!(event.kind, Kind::StageFailed | Kind::Cancelled) {
        record = record.with_outcome(false, true);
    }
    if let Some(writes) = workflow_stage_writes(&event.detail) {
        record = record.with_writes(writes);
    }
    Some(record)
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

/// Project a whole workflow run's `events.jsonl` into a topology trace.
///
/// This is the third tap. It runs at completion rather than per-event because
/// `WorkflowEventLog::emit` lives in `archon-workflow`, which depends on
/// exactly one Archon crate (`archon-llm`) and must not grow an edge onto the
/// binary to gain a callback — that thinness is why its persistence is
/// file-based in the first place. Replaying the file it already writes gets the
/// same records with no new dependency.
///
/// Returns the number of events projected. A missing or unreadable log is not
/// an error; it means the run wrote nothing worth folding.
pub(crate) fn project_workflow_run(
    project_root: &Path,
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
) -> usize {
    let Some(trace) = AmbientTrace::open(project_root, run_id, run_id).ok() else {
        return 0;
    };

    let path = store.events_path(run_id);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return 0;
    };

    // Only complete lines. `WorkflowStore::append_event_line` writes the body
    // and the newline as two separate `write_all` calls, so a concurrent reader
    // genuinely can catch a line mid-write there — unlike our own trace, which
    // writes both in one call.
    let complete = match contents.rfind('\n') {
        Some(index) => &contents[..=index],
        None => "",
    };

    let mut records = Vec::new();
    let mut origin_run_id = run_id.to_string();
    for line in complete.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<archon_workflow::WorkflowEvent>(line) else {
            continue;
        };
        if !event.run_id.is_empty() {
            origin_run_id = event.run_id.clone();
        }
        if let Some(record) = workflow_trace_record(run_id, &event) {
            records.push(record);
        }
    }

    for record in &records {
        trace.record(record.clone());
    }

    // Declare the reconstruction explicitly rather than letting the fold build
    // it. The fold's fallback origin is `Session`, and a workflow run that
    // reported itself as a session would break the corpus join on `run_id` —
    // which is the whole point of recording an origin.
    if !records.is_empty() {
        let graph = archon_topology::reconstruct::reconstruct_graph(
            run_id,
            GraphOrigin::Workflow {
                run_id: origin_run_id,
            },
            &records,
        );
        trace.declare_graph(&graph);
    }

    records.len()
}

/// Node id for a subagent spawned by a tool call.
fn spawn_node_id(tool_use_id: &str, attempt: u32) -> String {
    if tool_use_id.is_empty() {
        format!("spawn-{attempt}")
    } else {
        format!("spawn-{tool_use_id}-{attempt}")
    }
}

/// The `PermissionLevel` a tool declared, mapped onto the IR's class.
///
/// `Dangerous` maps to `Irreversible` rather than `Risky`: milestone 3 gates on
/// irreversibility, and under-classifying there is the failure that matters.
fn permission_class(level: PermissionLevel) -> PermissionClass {
    match level {
        PermissionLevel::Safe => PermissionClass::Safe,
        PermissionLevel::Risky => PermissionClass::Risky,
        PermissionLevel::Dangerous => PermissionClass::Irreversible,
    }
}

/// Files a tool call wrote, read out of its input.
///
/// Only for tools known to write. A `Read` also carries `file_path`, and
/// recording that as a write would manufacture write conflicts out of nothing.
fn written_paths(tool_name: &str, input: &serde_json::Value) -> Vec<WriteTarget> {
    if !WRITING_TOOLS.contains(&tool_name) {
        return Vec::new();
    }
    let Some(fields) = input.as_object() else {
        return Vec::new();
    };
    let mut targets: Vec<WriteTarget> = WRITE_PATH_KEYS
        .iter()
        .filter_map(|key| fields.get(*key))
        .filter_map(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(|path| WriteTarget::Path(normalize_write_path(path)))
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// Write targets are compared by exact string
/// (`TaskGraph::write_conflicts`), so separators must agree or two writes to
/// the same file look unrelated. Absolute prefixes are left alone: stripping
/// them needs a project root this function does not have, and an over-long key
/// under-reports conflicts rather than inventing them.
fn normalize_write_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// The agent type a subagent-spawning tool call named, if any.
fn subagent_type(input: &serde_json::Value) -> Option<String> {
    let fields = input.as_object()?;
    ["subagent_type", "agent_type", "agent", "type"]
        .iter()
        .filter_map(|key| fields.get(*key))
        .filter_map(serde_json::Value::as_str)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// Stage identifier out of a workflow event's untyped `detail` payload.
fn workflow_stage_id(detail: &serde_json::Value) -> Option<String> {
    let fields = detail.as_object()?;
    ["stage", "stage_id", "id", "name"]
        .iter()
        .filter_map(|key| fields.get(*key))
        .filter_map(serde_json::Value::as_str)
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// Declared write targets out of a workflow event's `detail` payload.
fn workflow_stage_writes(detail: &serde_json::Value) -> Option<Vec<WriteTarget>> {
    let fields = detail.as_object()?;
    let values = ["target_files", "expected_target_files", "writes"]
        .iter()
        .find_map(|key| fields.get(*key))
        .and_then(serde_json::Value::as_array)?;
    let mut targets: Vec<WriteTarget> = values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter(|path| !path.is_empty())
        .map(|path| WriteTarget::Path(normalize_write_path(path)))
        .collect();
    if targets.is_empty() {
        return None;
    }
    targets.sort();
    targets.dedup();
    Some(targets)
}

/// RFC3339 timestamp. `archon-topology` has no clock dependency, so the caller
/// supplies one; this is that caller.
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests;
