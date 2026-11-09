//! Attribute located files to declared writes.

use crate::task_universe::WorkflowV2TaskUniverse;

/// Who answers for a baseline failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ownership {
    /// The current task's: it declares the file.
    Current,
    /// Another task in the universe declares the file; routed to it.
    Other(String),
    /// No task declares the file: the current task's by default, and the
    /// first branch in a wave to meet it keeps it.
    Unowned,
}

/// Whether `task_id`'s declared writes cover `file` — a declared file, or a
/// declared directory above it.
pub(crate) fn task_owns(universe: &WorkflowV2TaskUniverse, task_id: &str, file: &str) -> bool {
    crate::v2::script::declared_writes(universe, task_id)
        .iter()
        .any(|declared| covers(declared, file))
}

fn covers(declared: &str, file: &str) -> bool {
    let declared = declared.trim_end_matches('/');
    declared == file
        || file
            .strip_prefix(declared)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Who answers for a failure in `file`: the current branch when its tasks
/// or its declared targets cover the file, else the first other task in
/// universe order that declares it, else nobody.
pub(crate) fn ownership(
    universe: Option<&WorkflowV2TaskUniverse>,
    own_task_ids: &[String],
    own_targets: &[String],
    file: &str,
) -> Ownership {
    if own_targets.iter().any(|target| covers(target, file)) {
        return Ownership::Current;
    }
    let Some(universe) = universe else {
        return Ownership::Unowned;
    };
    if own_task_ids
        .iter()
        .any(|task| task_owns(universe, task, file))
    {
        return Ownership::Current;
    }
    universe
        .tasks
        .iter()
        .filter(|task| !own_task_ids.contains(&task.canonical_task_id))
        .find(|task| task_owns(universe, &task.canonical_task_id, file))
        .map(|task| Ownership::Other(task.canonical_task_id.clone()))
        .unwrap_or(Ownership::Unowned)
}

#[cfg(test)]
#[path = "test_baseline_owner_tests.rs"]
mod tests;
