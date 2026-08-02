//! Fixtures shared by the fold's test suites.
//!
//! Owns the temporary project and its two guarded stores, the sample trace the
//! suites fold, and the row counters. The suites are split by what they assert:
//! [`fold_write`] on what one fold puts where, [`idempotence`] on what a repeat
//! fold must not do, [`metrics`] on the pure derivations, and [`hot_path`] on
//! the invariant that recording never touches a database.

mod fold_write;
mod hot_path;
mod idempotence;
mod metrics;

use std::path::Path;
use std::sync::Arc;

use archon_topology::ir::WriteTarget;
use archon_topology::trace::{TopologyPaths, TraceKind, TraceRecord, TraceWriter};
use cozo::{DbInstance, ScriptMutability};

use super::{ensure_topology_schema, fold_graph, fold_pending_blocking, topology_db_path};

/// The Cozo script poison and the ambient trace slot are both process-global,
/// so every test here serializes on the shared topology test lock.
use crate::command::topology_trace::test_lock as store_lock;

struct Fixture {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    paths: TopologyPaths,
    topology_db: Arc<DbInstance>,
    learning_db: Arc<DbInstance>,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    std::fs::create_dir_all(root.join(".archon")).unwrap();

    let topology_db = open_db(&topology_db_path(&root));
    let learning_db = open_db(&root.join(".archon").join("learning-state.db"));

    Fixture {
        paths: TopologyPaths::for_project(&root),
        _temp: temp,
        root,
        topology_db,
        learning_db,
    }
}

fn open_db(path: &Path) -> Arc<DbInstance> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let path = path.to_string_lossy().to_string();
    archon_cozo::open_sqlite_guarded_instance(
        &path,
        "open topology test store",
        archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .unwrap()
    .db_arc()
}

/// Write a small trace: a root that spawned two children, one of which failed.
fn write_sample_trace(paths: &TopologyPaths, graph_id: &str) -> TraceWriter {
    let writer = paths.writer(graph_id).unwrap();
    for record in [
        TraceRecord::new("2026-08-02T00:00:00Z", graph_id, TraceKind::AgentSpawned)
            .with_node("child-a")
            .with_parent("turn")
            .with_agent("worker"),
        TraceRecord::new("2026-08-02T00:00:01Z", graph_id, TraceKind::AgentSpawned)
            .with_node("child-b")
            .with_parent("turn")
            .with_agent("verifier"),
        TraceRecord::new("2026-08-02T00:00:02Z", graph_id, TraceKind::ToolAttempt)
            .with_node("child-a")
            .with_tool("Write")
            .with_attempt(1)
            .with_writes(vec![WriteTarget::Path("src/a.rs".into())])
            .with_duration_ms(120),
        TraceRecord::new("2026-08-02T00:00:03Z", graph_id, TraceKind::NodeFinished)
            .with_node("child-a"),
        TraceRecord::new("2026-08-02T00:00:04Z", graph_id, TraceKind::NodeFinished)
            .with_node("child-b")
            .with_outcome(false, true),
    ] {
        writer.append(&record).unwrap();
    }
    writer
}

fn count_learning_rows(db: &DbInstance) -> usize {
    archon_learning::store::list_all_learning_events(db)
        .unwrap()
        .iter()
        .filter(|event| {
            event.event_type == archon_learning::models::LearningEventType::TopologyOutcome
        })
        .count()
}

/// Count rows in a topology relation.
///
/// The projection must name **every** key column. Cozo query results are sets,
/// so projecting `topology_node` on `graph_id` alone collapses all of a graph's
/// nodes into one row and silently reports 1 no matter how many were written.
fn count_topology_rows(db: &DbInstance, relation: &str) -> usize {
    let projection = match relation {
        "topology_node" => "graph_id, node_id",
        _ => "graph_id",
    };
    db.run_script(
        &format!("?[{projection}] := *{relation}{{{projection}}}"),
        Default::default(),
        ScriptMutability::Immutable,
    )
    .map(|rows| rows.rows.len())
    .unwrap_or(0)
}
