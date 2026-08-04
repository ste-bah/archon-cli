//! Where the topology store lives, and the relations it holds.
//!
//! Owns the store's path resolution and the `:create` DDL for the three
//! topology relations plus the `task_hash` index. Nothing here reads or writes
//! a row — that is [`super::rows`] and [`super::learning_summary`].

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

/// Default file name for the topology store, relative to `.archon`.
pub(crate) const TOPOLOGY_DB_FILE: &str = "topology.db";

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
