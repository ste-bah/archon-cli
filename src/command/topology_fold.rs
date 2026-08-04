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
//! # Layout
//!
//! This file owns the orchestration — what a fold *is* and in what order it
//! happens. The pieces live beside it:
//!
//! - [`schema`] — where the store lives and its relation DDL.
//! - [`derive`] — graph plus trace into metrics; pure, no I/O.
//! - [`labels`] — how IR enums are spelled on the wire.
//! - [`rows`] — the one batched transaction that writes the three relations.
//! - [`learning_summary`] — the single shared-store row per graph.
//! - [`workflow_learning`] — the fold's **second input**: a workflow run's
//!   learning records, routed by the spec's `learning_hooks` into
//!   `LearningIntegration`. This is the consumer half of the bridge that
//!   `archon-workflow` cannot complete on its own (L3).
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

mod derive;
mod labels;
mod learning_summary;
mod rows;
mod schema;
/// Crate-visible so a plan-inspection test can route a hook list through the
/// real `plan_dispatch` rather than through a mirrored copy of
/// `INTEGRATION_HOOKS`. Asserting a derived hook against a second hand-kept
/// list of routable names proves the two lists agree, not that the fold routes
/// anything — which is the closed loop that let `learning_hooks` be dead for as
/// long as it was.
pub(crate) mod workflow_learning;

pub(crate) use workflow_learning::bridge_workflow_learning;

use std::path::Path;

use anyhow::{Context, Result};
use archon_topology::ir::GraphOrigin;
use archon_topology::trace::{TopologyPaths, read_trace};
use cozo::DbInstance;

use derive::derive;
use learning_summary::write_learning_summary;
use rows::write_topology_rows;

use schema::{ensure_topology_schema, topology_db_path};

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
pub(crate) fn open_store(path: &Path, label: &str) -> Result<std::sync::Arc<DbInstance>> {
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

#[cfg(test)]
mod tests;
