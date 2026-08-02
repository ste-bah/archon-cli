//! The batched write: three topology relations, one transaction.
//!
//! Owns the parameter binding and the multi-block CozoScript that puts
//! `topology_graph`, `topology_node`, and `topology_outcome` through a single
//! guarded call. One write lock acquisition per fold, regardless of node count —
//! never one write per node.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::derive::{FoldedGraph, wall_clock_ms};
use super::labels::{origin_ids, origin_label, permission_label, role_label, write_target_label};

/// Write all three topology relations in **one** guarded transaction.
///
/// One multi-block CozoScript, one `run_bound_script_guarded` call, one write
/// lock acquisition, regardless of node count. Never one write per node.
pub(super) fn write_topology_rows(
    db: &DbInstance,
    graph_id: &str,
    folded: &FoldedGraph,
) -> Result<()> {
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

fn int(value: usize) -> DataValue {
    DataValue::from(i64::try_from(value).unwrap_or(i64::MAX))
}
