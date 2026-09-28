//! Batch E: a refused project-input landing is a HIGH gap the third pass
//! owes its tasks.
//!
//! A landing that refused its branch's project-data changes
//! (`patch_apply::project_inputs_apply`: a stale baseline, a path no landing
//! may write, a failure part way) logged the refusal in the run's
//! append-only project-input log. The data the branch produced is not in the
//! project root, and only a new branch of the same task -- seeded with the
//! project's CURRENT data -- can produce it again. So each refusal not
//! answered since is owed, per task, as the host's restatement of that task
//! (its id is the task's), which the third pass plans as a write round of
//! the task's own files (`residual_third_pass`: an id that is a task's joins
//! that task's round). Answered: a later landing applied the same path for
//! the same task. Bounded like every owed gap: only log lines decided
//! before `cut`, the start of the pass's first round, so the plan never
//! moves once its rounds run.

use std::collections::{BTreeMap, BTreeSet};

use crate::v2::WorkflowV2ResultStore;
use crate::v2::script::residual_plan::{Residual, ResidualSeverity};
use crate::write_coordinator::patch_apply::{ProjectInputLanding, run_project_input_landings};

/// Id prefix of the recorder of a refused project-input landing.
pub const PROJECT_INPUT_RECORDER_PREFIX: &str = "project-inputs:";

/// The refused project-input landings no later landing answered, one gap per
/// (landing, task).
pub(crate) fn refused_input_gaps(store: &WorkflowV2ResultStore, cut: Option<i64>) -> Vec<Residual> {
    let Some(run_root) = store.root().parent() else {
        return Vec::new();
    };
    let lines = match run_project_input_landings(run_root) {
        Ok(lines) => lines,
        // Never read as "nothing refused": the gate weighs the unreadable
        // log itself (it names no task, so no pass can plan it).
        Err(error) => return vec![unreadable(&error)],
    };
    let lines: Vec<&ProjectInputLanding> = lines
        .iter()
        .filter(|line| cut.is_none_or(|at| line.at < at))
        .collect();
    let answered = |refusal: &ProjectInputLanding, task: &str| {
        lines.iter().any(|line| {
            line.at > refusal.at
                && line.landed()
                && line.path == refusal.path
                && line.task_ids.iter().any(|id| id == task)
        })
    };
    let mut gaps = Vec::new();
    // (landing, task) -> the refused paths and the first reason given.
    let mut owed: BTreeMap<(String, String, String), (BTreeSet<String>, String)> = BTreeMap::new();
    for refusal in lines.iter().filter(|line| line.refused()) {
        // A tracked input the host could not bring in step is no task's to
        // fix by re-running anything: it is weighed at the gate, unplanned.
        if refusal.outcome != "refused" {
            if !lines
                .iter()
                .any(|line| line.at > refusal.at && line.landed() && line.path == refusal.path)
            {
                gaps.push(host_gap(
                    &format!("{}/{}", refusal.stage_id, refusal.item_id),
                    &format!("{}: {}", refusal.path, refusal.reason),
                ));
            }
            continue;
        }
        for task in &refusal.task_ids {
            if answered(refusal, task) {
                continue;
            }
            let entry = owed
                .entry((
                    refusal.stage_id.clone(),
                    refusal.item_id.clone(),
                    task.clone(),
                ))
                .or_insert_with(|| (BTreeSet::new(), refusal.reason.clone()));
            entry.0.insert(refusal.path.clone());
        }
    }
    gaps.extend(owed.into_iter()
        .map(|((stage, item, task), (paths, reason))| Residual {
            recorded_by: format!("{PROJECT_INPUT_RECORDER_PREFIX}{stage}/{item}"),
            id: task.clone(),
            severity: ResidualSeverity::High,
            description: format!(
                "project data {task}'s branch `{item}` produced was NOT applied to the project root ({}): {reason}. Re-run the product's own data commands in this round's worktree, which holds the project's current data, so the result lands.",
                paths.into_iter().collect::<Vec<_>>().join(", ")
            ),
            files: Vec::new(),
            unit_tasks: std::iter::once(task).collect(),
            recorded_summary: String::new(),
            host_built: true,
        }));
    gaps
}

/// A HIGH gap of the host's own, naming no task: reported and weighed at
/// the final gate, never planned.
fn host_gap(recorder: &str, description: &str) -> Residual {
    Residual {
        recorded_by: format!("{PROJECT_INPUT_RECORDER_PREFIX}{recorder}"),
        id: "project_inputs_host".into(),
        severity: ResidualSeverity::High,
        description: description.to_string(),
        files: Vec::new(),
        unit_tasks: Default::default(),
        recorded_summary: String::new(),
        host_built: true,
    }
}

fn unreadable(error: &str) -> Residual {
    host_gap(
        "log",
        &format!(
            "the run's project-input log cannot be read, so no refused landing can be planned: {error}"
        ),
    )
}

#[cfg(test)]
#[path = "residual_project_inputs_tests.rs"]
mod tests;
