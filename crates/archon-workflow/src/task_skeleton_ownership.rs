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

/// A deliverable can carry only obligations claimed by its task.
pub(super) fn deliverable_claim_findings(skeleton: &TaskSkeleton) -> Vec<TaskSetFinding> {
    let mut findings = Vec::new();
    for (slot, task) in skeleton.tasks.iter().enumerate() {
        for (n, deliverable) in task.deliverable_contracts.iter().enumerate() {
            for (k, id) in deliverable.implements.iter().enumerate() {
                if !task.implements.contains(id) {
                    findings.push(TaskSetFinding {
                        identity: crate::defect::DeterministicDefect::new("deliverable_implements_unclaimed", skeleton_subject(&task.task_id, slot), format!("deliverable_contracts/{n}/implements/{k}")),
                        field: format!("deliverable_contracts/{n}/implements/{k}"),
                        message: format!("task '{}' deliverable '{}' implements '{}', which task '{}' does not claim in implements; add the id to the task's implements or remove it from the deliverable", task.task_id, deliverable.artifact_path, id, task.task_id),
                    });
                }
            }
        }
    }
    findings
}
