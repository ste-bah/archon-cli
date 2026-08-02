//! `Vec<Subtask>` → `archon_topology::TaskGraph`.
//!
//! This adapter lives in `archon-core` rather than in `archon-topology`
//! because `Subtask` is `archon-core`'s type. Putting it in the topology crate
//! would force an `archon-topology → archon-core` edge and invert the intended
//! layering — topology is the leaf that everything lowers *into*.
//!
//! The lowering is lossy by construction. `Subtask` carries
//! `{id, description, agent_type, dependencies, status, retries, max_retries}`
//! and nothing else: no dataflow, no write targets, no permission class. So
//! `consumes` and `writes` come out empty — meaning *unknown*, not *nothing*
//! — and every node is `Safe`. Analyses that reason from dataflow or writes
//! stay silent on a team graph until executors start declaring those, which is
//! the correct outcome rather than a gap.

use archon_topology::{GraphBudget, GraphOrigin, NodeRole, TaskGraph, TaskNode, TopologyError};

use super::events::Subtask;

/// Lower a decomposition into the topology IR.
///
/// The budget is derived from what `Subtask` actually knows: one agent per
/// subtask, and `max_rounds` from the deepest retry budget in the batch, since
/// a retry is the only repetition a team performs. `max_parallelism` is left at
/// the IR default; callers holding an `OrchestratorConfig` should overwrite it
/// with `max_concurrent`, which is the real cap.
#[must_use]
pub fn lower_subtasks(subtasks: &[Subtask], session_id: &str) -> TaskGraph {
    let max_rounds = subtasks
        .iter()
        .map(|subtask| subtask.max_retries.saturating_add(1))
        .max()
        .unwrap_or(1);

    TaskGraph {
        id: session_id.to_string(),
        origin: GraphOrigin::Team {
            session_id: session_id.to_string(),
        },
        nodes: subtasks
            .iter()
            .map(|subtask| TaskNode {
                depends_on: subtask.dependencies.clone(),
                agent: Some(subtask.agent_type.clone()),
                ..TaskNode::new(subtask.id.clone(), NodeRole::Work)
            })
            .collect(),
        budget: GraphBudget {
            max_agents: u32::try_from(subtasks.len()).unwrap_or(u32::MAX),
            max_rounds,
            ..GraphBudget::default()
        },
    }
}

/// Execution waves for a decomposition: tasks in the same wave may run
/// concurrently, and wave `n` may not start before wave `n-1` finishes.
///
/// Replaces the former `orchestrator::dag::build_dag_waves`, which was a second
/// independent DAG implementation alongside `archon-workflow`'s (finding O3).
/// Semantics are unchanged, including the order in which defects are reported:
/// an unknown dependency id before a cycle.
///
/// Error strings are reproduced verbatim from the deleted implementation so
/// nothing downstream that reads them changes behaviour.
pub fn build_dag_waves(subtasks: &[Subtask]) -> anyhow::Result<Vec<Vec<String>>> {
    lower_subtasks(subtasks, "").waves().map_err(subtask_error)
}

/// Lower, apply the real concurrency cap, and compute waves in one step.
///
/// The orchestrator calls this once per team run and schedules *every*
/// execution mode against the result. Before this, `Subtask::dependencies` was
/// honoured only in `ExecutionMode::Dag` (finding O1): `run_parallel` ignored
/// the field outright, and `Pipeline` synthesised dependencies at construction
/// that `run_sequential` never read.
pub fn plan(
    subtasks: &[Subtask],
    session_id: &str,
    max_parallelism: u32,
) -> anyhow::Result<(TaskGraph, Vec<Vec<String>>)> {
    let mut graph = lower_subtasks(subtasks, session_id);
    graph.budget.max_parallelism = max_parallelism;
    let waves = graph.waves().map_err(subtask_error)?;
    Ok((graph, waves))
}

fn subtask_error(error: TopologyError) -> anyhow::Error {
    match error {
        TopologyError::UnknownDependency { node, dependency } => {
            anyhow::anyhow!("subtask '{node}' depends on unknown subtask '{dependency}'")
        }
        TopologyError::Cycle => anyhow::anyhow!("dependency cycle detected in subtask graph"),
        // No counterpart in the old implementation, which silently let a later
        // duplicate id shadow an earlier one and produced nonsense waves.
        TopologyError::DuplicateNode { id } => {
            anyhow::anyhow!("duplicate subtask id '{id}' in subtask graph")
        }
    }
}
