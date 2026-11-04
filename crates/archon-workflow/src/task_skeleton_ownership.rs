//! One constraint ownership rule for skeleton freeze, body landing and coverage.
use super::*;
use std::collections::BTreeMap;

pub fn constraint_owner_findings(
    skeleton: &TaskSkeleton,
    constraints: &BTreeMap<String, String>,
) -> Vec<TaskSetFinding> {
    constraints.iter().filter(|(id, _)| !skeleton.tasks.iter()
        .any(|task| task.implements.contains(id)))
        .map(|(id, text)| TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("unowned_constraint", id, "implements"),
            field: "implements".into(),
            message: format!("PRD constraint '{id}' ({text}) is in no task's implements; add it to the implements of every task whose work it restricts, or of at least one task when it restricts none directly"),
        }).collect()
}
