//! Batch O: a task's declared write scope must be a real, literal set.
//!
//! - **No declared file (I3).** A task with no `## Files Expected to Change`
//!   heading, or one that lists nothing, and no shared-append target or
//!   deliverable contract either, silently became an empty write set: no
//!   branch could ever write its work, and nothing said so. It is a body
//!   finding, so the set gate re-authors that body.
//! - **Catch-all forbidden prose (hole 10).** A `Files Forbidden to Change`
//!   entry such as "everything else in the repository" names no path: the
//!   host cannot read it, so it either forbids nothing or, read as a word,
//!   defeats every grant. Each entry must name a literal path, directory,
//!   basename or glob.

use std::path::Path;

use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
use archon_workflow::task_universe::parsing::parse_task_file;

use crate::command::workflow_gate::{GateFinding, GateId};

/// The scope findings of one parsed task.
pub(super) fn inspect(task: &WorkflowV2TaskUniverseTask) -> Vec<String> {
    let id = &task.canonical_task_id;
    let mut findings = Vec::new();
    if task.files_expected_to_change.is_empty()
        && task.shared_append_target_files.is_empty()
        && task.deliverable_contracts.is_empty()
    {
        findings.push(format!(
            "task {id}: declares no file it changes -- the `## Files Expected to Change` heading is missing or lists nothing, and it has no shared-append target or deliverable contract -- so its write set would be empty and no branch could land its work; add the heading and list every file this task changes, each with its observation"
        ));
    }
    for entry in &task.files_forbidden_to_change {
        if prose_entry(entry) {
            findings.push(format!(
                "task {id}: Files Forbidden to Change entry \"{}\" names no path; replace it with the literal paths, directories, basenames or globs the task must not change",
                entry.trim()
            ));
        }
    }
    findings
}

/// An entry with no backticked span whose first word is no path shape.
fn prose_entry(entry: &str) -> bool {
    let trimmed = entry.trim();
    if trimmed.is_empty() || trimmed.contains('`') {
        return false;
    }
    let first = trimmed.split_whitespace().next().unwrap_or_default();
    !first.contains(['/', '.', '*'])
}

/// [`inspect`] over every task file under `root`, as body findings of the
/// file each names.
pub(super) fn set_findings(root: &Path) -> Vec<GateFinding> {
    let mut findings = Vec::new();
    for path in archon_workflow::task_universe::task_files_under(root).unwrap_or_default() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(task) = parse_task_file(&path, &raw) else {
            continue;
        };
        findings.extend(inspect(&task).into_iter().map(|text| {
            GateFinding::new(
                GateId::WorkflowLintTaskSet,
                text,
                &task.canonical_task_id,
                Some(path.clone()),
                archon_workflow::RemediationScope::Body,
            )
        }));
    }
    findings
}

#[cfg(test)]
#[path = "scope_declarations_tests.rs"]
mod tests;
