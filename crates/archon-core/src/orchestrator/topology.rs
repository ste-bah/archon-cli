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

use archon_tools::tool::PermissionLevel;
use archon_topology::{
    GraphBudget, GraphOrigin, NodeRole, PermissionClass, TaskGraph, TaskNode, TopologyError,
};

use super::events::Subtask;

/// `PermissionLevel` → [`PermissionClass`]. **The** runtime mapping.
///
/// Milestone 1 lowered `WorkflowSpec::permissions` best-effort against no
/// schema and fell through to `Safe`, which is fail-open and cannot carry a
/// safety invariant. Milestone 3 grounds the declared format in the enum that
/// already exists and that admission already receives on every tool call, and
/// this function is where the two meet. `archon-topology` cannot name
/// `PermissionLevel` — `archon-tools` is far outside its dependency budget — so
/// the tie is made here, in the one crate that depends on both, and asserted by
/// the conformance test below.
///
/// `Dangerous` maps to `Irreversible`, not to `Risky`. Milestone 3 gates on
/// irreversibility and under-classifying there is the failure that matters; the
/// command classifier's `Dangerous` set (`git push`, `rm -rf`, `sudo`,
/// `shutdown`) is exactly the design's list of irreversible effects.
#[must_use]
pub fn permission_class_for_level(level: PermissionLevel) -> PermissionClass {
    match level {
        PermissionLevel::Safe => PermissionClass::Safe,
        PermissionLevel::Risky => PermissionClass::Risky,
        PermissionLevel::Dangerous => PermissionClass::Irreversible,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The two ladders must not drift apart.
    ///
    /// `archon-topology` parses the declared spec-side format from strings and
    /// `archon-tools` classifies live tool calls into `PermissionLevel`. If
    /// those disagree, a stage authored `dangerous` and a tool classified
    /// `Dangerous` would admit differently, and milestone 3's invariant 3 would
    /// depend on which surface the action came from. Adding a `PermissionLevel`
    /// variant fails here rather than silently defaulting somewhere.
    #[test]
    fn the_declared_format_and_permission_level_agree_variant_for_variant() {
        for (level, declared) in [
            (PermissionLevel::Safe, "safe"),
            (PermissionLevel::Risky, "risky"),
            (PermissionLevel::Dangerous, "dangerous"),
        ] {
            let from_level = permission_class_for_level(level);
            assert_eq!(
                PermissionClass::from_declared(declared),
                Some(from_level),
                "declared {declared:?} must lower to the same class as {level:?}"
            );
            assert_eq!(
                from_level.as_declared(),
                declared,
                "{level:?} must print back as {declared:?}"
            );
        }
        assert_eq!(
            archon_topology::DECLARED_PERMISSION_LEVELS.len(),
            3,
            "PermissionLevel has three variants; the declared vocabulary must match"
        );
    }

    #[test]
    fn dangerous_is_irreversible_because_under_classifying_is_the_failure_that_matters() {
        assert_eq!(
            permission_class_for_level(PermissionLevel::Dangerous),
            PermissionClass::Irreversible
        );
    }
}
