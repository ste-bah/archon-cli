//! The batched fold: trace files in, two stores out, one writer.
//!
//! # Why it lives here and not in `archon-pipeline`
//!
//! The fold needs three things at once: the topology IR
//! (`archon-topology`), workflow run artifacts (`archon-workflow`), and the
//! learning stack (`archon-pipeline`). `archon-pipeline` does not depend on
//! `archon-workflow` and must not start — workflow is the deliberately thin,
//! provider-neutral leaf, which is *why* its persistence is file-based. Adding
//! that edge would invert the layering.
//!
//! The binary is already the composition root for the learning stack: every
//! `LearningIntegration` is constructed in `src/`
//! (`pipeline_support.rs`, `src/session/interactive_agent.rs`), and
//! `pipeline_support.rs` is already the workflow↔pipeline bridge. The fold sits
//! beside it. This also keeps `archon-topology` and `archon-workflow` free of
//! learning-stack dependencies, which milestone 1 requires.
//!
//! # Concurrency contract
//!
//! Normative, and all of it follows from the stores being SQLite-backed behind
//! a global write lock:
//!
//! - **One writer.** N workers produce traces; exactly one fold consumes them.
//!   Contention is O(1) in fleet size rather than O(N).
//! - **One transaction.** All three topology relations are written by a single
//!   multi-block CozoScript through one guarded call. Cozo's imperative blocks
//!   are atomic together — a failure in a later block rolls back an earlier one
//!   — so a partial fold cannot be observed.
//! - **Bulk rows to their own database file.** `.archon/topology.db`. The write
//!   lock key is per canonicalized path, so a separate file cannot contend with
//!   the knowledge, learning, or completion stores.
//! - **Exactly one row per graph in the shared learning store.** That single
//!   summary row is the deliberate exception to the isolation rule, and it must
//!   never become one row per node.
//! - **All mutations through the guard.** `run_bound_script_guarded`, never raw
//!   `db.run_script`. Reads go direct: `run_guarded_once` takes locks only for
//!   `Mutable` scripts, so reads parallelize freely and taking the guard for
//!   them would only add latency.
//! - **Async callers must not call [`fold_graph`] directly.** It is synchronous
//!   and the guard's sync retry loop sleeps on `thread::sleep`, ~19 seconds
//!   worst case — a runtime stall on a tokio worker. Use
//!   [`fold_pending_blocking`] from `spawn_blocking`.
//!
//! # Idempotence
//!
//! Re-folding the same trace must not double-count. Two mechanisms, belt and
//! braces:
//!
//! - Every row is keyed by `graph_id` (and `node_id`), and every write is
//!   `:put`, which is upsert. Replaying a fold overwrites rather than appends.
//! - The `learning_events` row id is derived from the graph id, so the one
//!   summary row is also an upsert rather than an insert.
//! - The `ingested` marker is written **last**, so a crash mid-fold replays
//!   (harmless, because of the two above) rather than losing the graph.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use archon_topology::ir::{GraphOrigin, NodeRole, PermissionClass, TaskGraph, WriteTarget};
use archon_topology::reconstruct::{observed_retries, reconstruct_graph};
use archon_topology::trace::{TopologyPaths, TraceKind, TraceReadout, TraceRecord, read_trace};
use cozo::{DataValue, DbInstance, ScriptMutability};

/// `event_type` written into `learning_events` for the per-graph summary.
///
/// **Not** the design's `"topology_outcome"`. `learning_events.event_type` is
/// not a free string: it is written from
/// `archon_learning::models::LearningEventType::as_str`, a closed enum whose
/// twenty-three existing variants are all PascalCase, and both
/// `learning_events:by_type_created_at` index queries in the tree match on that
/// spelling. A snake_case value would parse back as `None` from
/// `LearningEventType::from_str` and would sort oddly next to its siblings. The
/// variant is `TopologyOutcome`; the wire form is this constant.
pub(crate) const TOPOLOGY_OUTCOME_EVENT_TYPE: &str = "TopologyOutcome";

/// Default file name for the topology store, relative to `.archon`.
pub(crate) const TOPOLOGY_DB_FILE: &str = "topology.db";

/// What one fold did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FoldOutcome {
    pub graph_id: String,
    pub nodes_written: usize,
    /// `learning_events` rows written. Exactly 0 or 1, never more.
    pub learning_rows_written: usize,
    /// True when the graph was already marked ingested and nothing was written.
    pub already_ingested: bool,
    /// True when the trace had a partial trailing line, which was skipped.
    pub truncated_trace: bool,
    /// True when no `graph.json` existed and a skeleton was reconstructed.
    pub reconstructed: bool,
}

impl FoldOutcome {
    fn skipped(graph_id: &str) -> Self {
        Self {
            graph_id: graph_id.to_string(),
            nodes_written: 0,
            learning_rows_written: 0,
            already_ingested: true,
            truncated_trace: false,
            reconstructed: false,
        }
    }
}

/// `<project_root>/.archon/topology.db`.
pub(crate) fn topology_db_path(project_root: &Path) -> std::path::PathBuf {
    project_root.join(".archon").join(TOPOLOGY_DB_FILE)
}

/// Create the topology relations. Idempotent; safe to call on every fold.
pub(crate) fn ensure_topology_schema(db: &DbInstance) -> Result<()> {
    // `depends_on` and `writes` are `String`, not the design sketch's `Json`.
    // Nothing in the tree uses a Cozo `Json` column; the established convention
    // across `agent_performance_ledger`, `provider_runtime_events`, and the
    // rest is a `String` holding serialized JSON. Following the sketch here
    // would have made this the only relation a caller must handle differently.
    for script in [
        r#":create topology_graph {
            graph_id: String =>
            origin: String,
            task_hash: String default "",
            run_id: String default "",
            session_id: String default "",
            node_count: Int default 0,
            span: Int default 0,
            work: Int default 0,
            max_parallelism_used: Int default 0,
            budget_max_parallelism: Int default 0,
            reconstructed: Bool default false,
            created_at: String,
        }"#,
        r#":create topology_node {
            graph_id: String, node_id: String =>
            role: String,
            agent: String default "",
            depends_on_json: String default "[]",
            writes_json: String default "[]",
            permission: String default "safe",
            duration_ms: Int default -1,
            retries: Int default 0,
            outcome: String default "unknown",
        }"#,
        r#":create topology_outcome {
            graph_id: String =>
            verified: Bool default false,
            human_corrections: Int default 0,
            cost_usd: Float default -1.0,
            wall_clock_ms: Int default -1,
            failure_class: String default "",
            nodes_failed: Int default 0,
            retries_total: Int default 0,
            write_conflicts: Int default 0,
        }"#,
        "::index create topology_graph:by_task_hash {task_hash}",
    ] {
        create_relation(db, script)?;
    }
    Ok(())
}

/// Run a `:create`, tolerating "already exists". Mirrors
/// `archon_learning::schema::run_create`.
fn create_relation(db: &DbInstance, script: &str) -> Result<()> {
    match archon_cozo::run_bound_script_guarded(
        db,
        script,
        BTreeMap::new(),
        ScriptMutability::Mutable,
        "topology schema creation",
    ) {
        Ok(_) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            if archon_learning::errors::COZO_RELATION_ALREADY_EXISTS
                .iter()
                .any(|phrase| message.contains(phrase))
                || message.contains("index already exists")
                || message.contains("Index") && message.contains("already exists")
            {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "topology schema creation failed: {message}"
                ))
            }
        }
    }
}

/// Everything the fold derived from one graph's trace, before it is written.
///
/// Split out from the write so it can be computed and asserted on without a
/// database, and so the write stays a single obvious transaction.
#[derive(Debug, Clone)]
pub(crate) struct FoldedGraph {
    pub graph: TaskGraph,
    pub task_hash: String,
    pub node_count: usize,
    pub span: usize,
    pub work: usize,
    pub max_parallelism_used: usize,
    pub retries: BTreeMap<String, usize>,
    pub outcomes: BTreeMap<String, String>,
    pub durations: BTreeMap<String, u64>,
    pub nodes_failed: usize,
    pub retries_total: usize,
    pub write_conflicts: usize,
    pub verified: bool,
    pub failure_class: String,
    pub truncated_trace: bool,
    pub reconstructed: bool,
}

/// Derive metrics from a graph and its trace. Pure; no I/O.
///
/// `fallback_origin` labels a reconstruction. It is ignored when `declared` is
/// present, because a declared graph already knows what it is.
pub(crate) fn derive(
    graph_id: &str,
    fallback_origin: GraphOrigin,
    declared: Option<TaskGraph>,
    readout: &TraceReadout,
    goal_text: &str,
) -> FoldedGraph {
    let reconstructed = declared.is_none();
    let graph =
        declared.unwrap_or_else(|| reconstruct_graph(graph_id, fallback_origin, &readout.records));

    // The analyses are total on an acyclic graph and fallible otherwise. A
    // malformed graph must not strand its trace, so a failure degrades the
    // metric rather than the fold: span falls back to node count, which is the
    // correct upper bound, and occupancy to 0, which claims nothing.
    let span = graph
        .critical_path()
        .map(|path| path.span())
        .unwrap_or(graph.len());
    let max_parallelism_used = graph
        .parallelism_profile()
        .map(|profile| profile.peak_width)
        .unwrap_or(0);
    let write_conflicts = graph.write_conflicts().map(|c| c.len()).unwrap_or(0);

    let retries = observed_retries(&readout.records);
    let outcomes = node_outcomes(&readout.records);
    let durations = node_durations(&readout.records);

    let nodes_failed = outcomes.values().filter(|state| *state == "failed").count();
    let retries_total = retries.values().sum();
    let verified = graph.nodes.iter().any(|node| node.role == NodeRole::Verify)
        && nodes_failed == 0
        && !readout.records.is_empty();

    FoldedGraph {
        task_hash: archon_topology::task_hash(goal_text),
        node_count: graph.len(),
        span,
        work: graph.len(),
        max_parallelism_used,
        nodes_failed,
        retries_total,
        write_conflicts,
        verified,
        failure_class: failure_class(nodes_failed, readout),
        truncated_trace: readout.truncated_tail,
        reconstructed,
        retries,
        outcomes,
        durations,
        graph,
    }
}

/// Fold one graph.
///
/// **Synchronous, and it can park a thread.** See the module note; async
/// callers go through [`fold_pending_blocking`].
///
/// `learning_db` is optional: absent a learning store the fold still writes its
/// own store and does not error. That is deliberate — the topology corpus is
/// useful on its own, and a missing learning store is a configuration state,
/// not a failure.
pub(crate) fn fold_graph(
    paths: &TopologyPaths,
    graph_id: &str,
    session_id: &str,
    goal_text: &str,
    topology_db: &DbInstance,
    learning_db: Option<&DbInstance>,
    workspace_id: &str,
) -> Result<FoldOutcome> {
    if paths.is_ingested(graph_id) {
        return Ok(FoldOutcome::skipped(graph_id));
    }

    let readout = read_trace(&paths.trace_jsonl(graph_id))
        .with_context(|| format!("read topology trace for {graph_id}"))?;
    let declared = paths
        .read_graph(graph_id)
        .with_context(|| format!("read topology graph for {graph_id}"))?;
    let folded = derive(
        graph_id,
        GraphOrigin::Session {
            session_id: session_id.to_string(),
        },
        declared,
        &readout,
        goal_text,
    );

    ensure_topology_schema(topology_db)?;
    write_topology_rows(topology_db, graph_id, &folded)?;

    let mut learning_rows_written = 0;
    if let Some(learning_db) = learning_db {
        write_learning_summary(learning_db, graph_id, workspace_id, &folded)?;
        learning_rows_written = 1;
    }

    // Last. A crash before this replays the fold, which is idempotent; a crash
    // after it would lose the graph.
    paths
        .mark_ingested(graph_id, &folded.task_hash)
        .with_context(|| format!("mark topology graph {graph_id} ingested"))?;

    Ok(FoldOutcome {
        graph_id: graph_id.to_string(),
        nodes_written: folded.node_count,
        learning_rows_written,
        already_ingested: false,
        truncated_trace: folded.truncated_trace,
        reconstructed: folded.reconstructed,
    })
}

/// Fold every graph that has a trace and no `ingested` marker.
///
/// Blocking. Call from `spawn_blocking` on an async path. Errors on one graph
/// do not abort the rest: a single corrupt trace must not block the corpus.
pub(crate) fn fold_pending_blocking(
    paths: &TopologyPaths,
    session_id: &str,
    goal_text: &str,
    topology_db: &DbInstance,
    learning_db: Option<&DbInstance>,
    workspace_id: &str,
) -> Vec<FoldOutcome> {
    let graph_ids = match paths.list_graph_ids() {
        Ok(ids) => ids,
        Err(error) => {
            tracing::debug!(%error, "topology fold could not list graphs");
            return Vec::new();
        }
    };

    graph_ids
        .into_iter()
        .filter(|graph_id| !paths.is_ingested(graph_id))
        .filter_map(|graph_id| {
            match fold_graph(
                paths,
                &graph_id,
                session_id,
                goal_text,
                topology_db,
                learning_db,
                workspace_id,
            ) {
                Ok(outcome) => Some(outcome),
                Err(error) => {
                    tracing::warn!(%error, %graph_id, "topology fold failed");
                    None
                }
            }
        })
        .collect()
}

/// Open both stores and fold every pending graph for a project.
///
/// The convenience entry point for "graph completion or idle timer". Blocking
/// and best-effort: it opens the stores, folds what it can, and returns what it
/// did. A store that will not open is logged and skipped rather than raised —
/// the corpus is an observation, and failing a user's turn to record one would
/// be the wrong trade.
///
/// **Call from `spawn_blocking`.** The guard's sync retry loop sleeps on
/// `thread::sleep`, which on a tokio worker is a runtime stall.
pub(crate) fn fold_project_pending_blocking(
    project_root: &Path,
    session_id: &str,
    goal_text: &str,
    workspace_id: &str,
) -> Vec<FoldOutcome> {
    let paths = TopologyPaths::for_project(project_root);
    if paths
        .list_graph_ids()
        .map(|ids| ids.is_empty())
        .unwrap_or(true)
    {
        return Vec::new();
    }

    let topology_db = match open_store(&topology_db_path(project_root), "topology") {
        Ok(db) => db,
        Err(error) => {
            tracing::warn!(%error, "topology store unavailable; skipping fold");
            return Vec::new();
        }
    };
    // A missing learning store is a configuration state, not a failure: the
    // fold still writes its own store.
    let learning_db = open_store(
        &project_root.join(".archon").join("learning-state.db"),
        "learning",
    )
    .map_err(
        |error| tracing::debug!(%error, "learning store unavailable; topology summary skipped"),
    )
    .ok();

    fold_pending_blocking(
        &paths,
        session_id,
        goal_text,
        &topology_db,
        learning_db.as_deref(),
        workspace_id,
    )
}

/// Open a guarded sqlite store, registering its guard config so
/// `run_bound_script_guarded` can resolve it.
fn open_store(path: &Path, label: &str) -> Result<std::sync::Arc<DbInstance>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path = path.to_string_lossy().to_string();
    Ok(archon_cozo::open_sqlite_guarded_instance(
        &path,
        &format!("open {label} store at {path}"),
        archon_cozo::CozoGuardConfig::for_db_path(&path),
    )?
    .db_arc())
}

/// Write all three topology relations in **one** guarded transaction.
///
/// One multi-block CozoScript, one `run_bound_script_guarded` call, one write
/// lock acquisition, regardless of node count. Never one write per node.
fn write_topology_rows(db: &DbInstance, graph_id: &str, folded: &FoldedGraph) -> Result<()> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let (run_id, session_id) = origin_ids(&folded.graph.origin);

    let mut params = BTreeMap::new();
    params.insert(
        "graph".to_string(),
        DataValue::List(vec![DataValue::List(vec![
            DataValue::from(graph_id),
            DataValue::from(origin_label(&folded.graph.origin)),
            DataValue::from(folded.task_hash.as_str()),
            DataValue::from(run_id.as_str()),
            DataValue::from(session_id.as_str()),
            int(folded.node_count),
            int(folded.span),
            int(folded.work),
            int(folded.max_parallelism_used),
            DataValue::from(i64::from(folded.graph.budget.max_parallelism)),
            DataValue::Bool(folded.reconstructed),
            DataValue::from(created_at.as_str()),
        ])]),
    );
    params.insert(
        "nodes".to_string(),
        DataValue::List(node_rows(graph_id, folded)),
    );
    params.insert(
        "outcome".to_string(),
        DataValue::List(vec![DataValue::List(vec![
            DataValue::from(graph_id),
            DataValue::Bool(folded.verified),
            // Human corrections and cost are not observable from the trace.
            // `-1` is the tree's convention for "not measured" (see
            // `agent_performance_ledger.quality_score`); reporting 0 would be a
            // measurement claim this fold cannot make.
            DataValue::from(0i64),
            DataValue::from(-1.0f64),
            int(wall_clock_ms(folded)),
            DataValue::from(folded.failure_class.as_str()),
            int(folded.nodes_failed),
            int(folded.retries_total),
            int(folded.write_conflicts),
        ])]),
    );

    // Three `:put`s, one transaction. Verified empirically against
    // cozo-ce 0.7.13: a failure in a later block rolls back the earlier ones.
    let script = "\
{ ?[graph_id, origin, task_hash, run_id, session_id, node_count, span, work, \
max_parallelism_used, budget_max_parallelism, reconstructed, created_at] <- $graph \
:put topology_graph { graph_id => origin, task_hash, run_id, session_id, node_count, \
span, work, max_parallelism_used, budget_max_parallelism, reconstructed, created_at } }
{ ?[graph_id, node_id, role, agent, depends_on_json, writes_json, permission, \
duration_ms, retries, outcome] <- $nodes \
:put topology_node { graph_id, node_id => role, agent, depends_on_json, writes_json, \
permission, duration_ms, retries, outcome } }
{ ?[graph_id, verified, human_corrections, cost_usd, wall_clock_ms, failure_class, \
nodes_failed, retries_total, write_conflicts] <- $outcome \
:put topology_outcome { graph_id => verified, human_corrections, cost_usd, \
wall_clock_ms, failure_class, nodes_failed, retries_total, write_conflicts } }";

    archon_cozo::run_bound_script_guarded(
        db,
        script,
        params,
        ScriptMutability::Mutable,
        "topology fold batch write",
    )
    .with_context(|| format!("write topology rows for {graph_id}"))?;
    Ok(())
}

fn node_rows(graph_id: &str, folded: &FoldedGraph) -> Vec<DataValue> {
    folded
        .graph
        .nodes
        .iter()
        .map(|node| {
            let depends_on =
                serde_json::to_string(&node.depends_on).unwrap_or_else(|_| "[]".to_string());
            let writes = serde_json::to_string(
                &node
                    .writes
                    .iter()
                    .map(write_target_label)
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_else(|_| "[]".to_string());
            DataValue::List(vec![
                DataValue::from(graph_id),
                DataValue::from(node.id.as_str()),
                DataValue::from(role_label(node.role)),
                DataValue::from(node.agent.as_deref().unwrap_or("")),
                DataValue::from(depends_on.as_str()),
                DataValue::from(writes.as_str()),
                DataValue::from(permission_label(node.permission)),
                folded
                    .durations
                    .get(&node.id)
                    .map_or(DataValue::from(-1i64), |ms| {
                        DataValue::from(i64::try_from(*ms).unwrap_or(i64::MAX))
                    }),
                int(folded.retries.get(&node.id).copied().unwrap_or(0)),
                DataValue::from(
                    folded
                        .outcomes
                        .get(&node.id)
                        .map_or("unknown", String::as_str),
                ),
            ])
        })
        .collect()
}

/// Write **one** summary row per graph into the shared `learning_events`
/// relation.
///
/// This is the deliberate exception to "bulk rows to their own file": it makes
/// topology visible to every existing `learning_events` consumer without any of
/// them changing. It is one write per graph and must never become one per node
/// — the whole point of the batched fold is that the shared store sees O(1)
/// writers.
///
/// The row id is derived from the graph id, so a repeat fold upserts the same
/// row rather than adding a second one.
fn write_learning_summary(
    db: &DbInstance,
    graph_id: &str,
    workspace_id: &str,
    folded: &FoldedGraph,
) -> Result<()> {
    archon_learning::schema::ensure_learning_schema(db)
        .context("ensure learning schema for topology summary")?;
    debug_assert_eq!(
        archon_learning::models::LearningEventType::TopologyOutcome.as_str(),
        TOPOLOGY_OUTCOME_EVENT_TYPE,
        "the documented wire form and the enum must not drift apart"
    );

    let signal = serde_json::json!({
        "graph_id": graph_id,
        "task_hash": folded.task_hash,
        "origin": origin_label(&folded.graph.origin),
        "node_count": folded.node_count,
        "span": folded.span,
        "work": folded.work,
        "max_parallelism_used": folded.max_parallelism_used,
        "budget_max_parallelism": folded.graph.budget.max_parallelism,
        "wave_widths": folded
            .graph
            .parallelism_profile()
            .map(|profile| profile.wave_widths)
            .unwrap_or_default(),
        "fan_out_widths": fan_out_widths(&folded.graph),
        "verifier_count": folded
            .graph
            .nodes
            .iter()
            .filter(|node| node.role == NodeRole::Verify)
            .count(),
        "verifier_independence": verifier_independence(&folded.graph),
        "gate_nodes": folded.graph.gate_nodes(),
        "ungated_irreversible": folded.graph.ungated_irreversible().unwrap_or_default(),
        "write_conflicts": folded.write_conflicts,
        "retries_total": folded.retries_total,
        "nodes_failed": folded.nodes_failed,
        "verified": folded.verified,
        "failure_class": folded.failure_class,
        "reconstructed": folded.reconstructed,
        "truncated_trace": folded.truncated_trace,
    });

    let event = archon_learning::models::LearningEvent {
        // Deterministic in the graph id: the idempotence of the whole fold
        // rests on this being an upsert key rather than a fresh uuid.
        event_id: format!("topology-outcome-{graph_id}"),
        workspace_id: workspace_id.to_string(),
        event_type: archon_learning::models::LearningEventType::TopologyOutcome,
        source_artifact_id: graph_id.to_string(),
        outcome_artifact_id: None,
        signal,
        // Confidence maps from verification strength: a graph with independent
        // verifiers and no failures is worth more than a bare reconstruction.
        confidence: confidence_from_verification(folded),
        provenance_record_id: String::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    archon_learning::store::insert_learning_event(db, &event)
        .context("insert topology_outcome learning event")
}

/// Confidence in the range `[0.1, 0.95]`.
///
/// A reconstructed skeleton is a weaker observation than a declared graph, and
/// a graph with independent verifiers that all passed is a stronger one.
fn confidence_from_verification(folded: &FoldedGraph) -> f32 {
    let mut confidence: f32 = if folded.reconstructed { 0.3 } else { 0.6 };
    if folded.verified {
        confidence += 0.2;
    }
    if folded.nodes_failed > 0 {
        confidence -= 0.15;
    }
    if folded.truncated_trace {
        confidence -= 0.1;
    }
    confidence.clamp(0.1, 0.95)
}

/// Widths of every fan-out in the graph: for each node, how many nodes depend
/// directly on it, wherever that is more than one.
fn fan_out_widths(graph: &TaskGraph) -> Vec<usize> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &graph.nodes {
        for dependency in &node.depends_on {
            *counts.entry(dependency.as_str()).or_default() += 1;
        }
    }
    let mut widths: Vec<usize> = counts.into_values().filter(|width| *width > 1).collect();
    widths.sort_unstable();
    widths
}

/// How many verifiers share no dependency with each other.
///
/// A crude proxy, and named as one. Three verifiers all fed by the same
/// producer are not three independent checks; three verifiers with disjoint
/// dependency sets plausibly are. Milestone 4 replaces this with something that
/// reasons about declared dataflow, which does not exist yet.
fn verifier_independence(graph: &TaskGraph) -> usize {
    let verifiers: Vec<&archon_topology::ir::TaskNode> = graph
        .nodes
        .iter()
        .filter(|node| node.role == NodeRole::Verify)
        .collect();
    let mut independent = 0;
    for (index, left) in verifiers.iter().enumerate() {
        let disjoint = verifiers.iter().enumerate().all(|(other, right)| {
            other == index
                || !left
                    .depends_on
                    .iter()
                    .any(|dependency| right.depends_on.contains(dependency))
        });
        if disjoint {
            independent += 1;
        }
    }
    independent
}

/// Terminal state per node, from the trace.
fn node_outcomes(records: &[TraceRecord]) -> BTreeMap<String, String> {
    let mut outcomes: BTreeMap<String, String> = BTreeMap::new();
    for record in records {
        if record.node_id.is_empty() || record.kind == TraceKind::Unknown {
            continue;
        }
        let state = match record.kind {
            TraceKind::NodeFinished if record.error => "failed",
            TraceKind::NodeFinished => "completed",
            TraceKind::NodeStarted | TraceKind::AgentSpawned => "started",
            TraceKind::ToolAttempt if record.blocked => "blocked",
            TraceKind::ToolAttempt if record.error => "errored",
            _ => continue,
        };
        // A terminal state is sticky: a later `started` from a retry must not
        // erase an earlier failure.
        let existing = outcomes.get(&record.node_id).map(String::as_str);
        if matches!(existing, Some("failed")) {
            continue;
        }
        if matches!(existing, Some("completed")) && state != "failed" {
            continue;
        }
        outcomes.insert(record.node_id.clone(), state.to_string());
    }
    outcomes
}

/// Longest observed duration per node.
fn node_durations(records: &[TraceRecord]) -> BTreeMap<String, u64> {
    let mut durations: BTreeMap<String, u64> = BTreeMap::new();
    for record in records {
        let (Some(duration), false) = (record.duration_ms, record.node_id.is_empty()) else {
            continue;
        };
        let entry = durations.entry(record.node_id.clone()).or_default();
        *entry = (*entry).max(duration);
    }
    durations
}

/// Sum of observed node durations, or `-1` when nothing reported one.
///
/// Not wall clock in the strict sense — the trace carries no start and end
/// stamp for the graph as a whole — and reported as a sum rather than
/// pretending otherwise.
fn wall_clock_ms(folded: &FoldedGraph) -> usize {
    if folded.durations.is_empty() {
        return 0;
    }
    folded
        .durations
        .values()
        .copied()
        .map(|ms| usize::try_from(ms).unwrap_or(usize::MAX))
        .sum()
}

fn failure_class(nodes_failed: usize, readout: &TraceReadout) -> String {
    if nodes_failed > 0 {
        return "node_failure".to_string();
    }
    if readout.records.iter().any(|record| record.blocked) {
        return "admission_blocked".to_string();
    }
    if readout.truncated_tail {
        return "trace_truncated".to_string();
    }
    String::new()
}

fn origin_label(origin: &GraphOrigin) -> &'static str {
    match origin {
        GraphOrigin::Workflow { .. } => "workflow",
        GraphOrigin::Team { .. } => "team",
        GraphOrigin::Session { .. } => "session",
    }
}

/// `(run_id, session_id)` — whichever the origin carries; the other is empty.
fn origin_ids(origin: &GraphOrigin) -> (String, String) {
    match origin {
        GraphOrigin::Workflow { run_id } => (run_id.clone(), String::new()),
        GraphOrigin::Team { session_id } | GraphOrigin::Session { session_id } => {
            (String::new(), session_id.clone())
        }
    }
}

fn role_label(role: NodeRole) -> String {
    match role {
        NodeRole::Plan => "plan".to_string(),
        NodeRole::Work => "work".to_string(),
        NodeRole::Verify => "verify".to_string(),
        NodeRole::Reduce => "reduce".to_string(),
        NodeRole::Tool => "tool".to_string(),
        NodeRole::Gate(kind) => format!("gate:{}", gate_label(kind)),
    }
}

fn gate_label(kind: archon_topology::ir::GateKind) -> &'static str {
    match kind {
        archon_topology::ir::GateKind::Human => "human",
        archon_topology::ir::GateKind::Checkpoint => "checkpoint",
    }
}

fn permission_label(permission: PermissionClass) -> &'static str {
    match permission {
        PermissionClass::Safe => "safe",
        PermissionClass::Risky => "risky",
        PermissionClass::Irreversible => "irreversible",
    }
}

fn write_target_label(target: &WriteTarget) -> String {
    match target {
        WriteTarget::Path(path) => format!("path:{path}"),
        WriteTarget::Artifact(key) => format!("artifact:{key}"),
    }
}

fn int(value: usize) -> DataValue {
    DataValue::from(i64::try_from(value).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests;
