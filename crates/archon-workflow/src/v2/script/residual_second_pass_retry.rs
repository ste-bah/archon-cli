//! Issue-118: a first-pass round its own judge refused over red tests it
//! could not write, planned again by the second pass.
//!
//! The host's demotion names the red tests on the judge's record
//! (`baseline_red_tests`). Each is resolved to its file, its parent
//! module's, and every file the host's own run on the judged tree saw it
//! fail in -- AT THE COMMIT THE JUDGE JUDGED, never the tree as it is now.
//! Those the round's tasks or granted files already cover were the round's
//! own to fix: they buy nothing. The rest become one gap the retry carries
//! beside the round's own gaps, granted: the round's own files, plus every
//! implicated file no task declares (when the host may open it); an
//! implicated file another task declares brings that task into the round.
//! The retry resolves the round's own gaps when it resolves (the gate reads
//! a gap resolved by ANY round that carried it), so a refused round never
//! stands on a verdict about tests it could not write.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use super::super::super::residual_paths::{expandable, owners};
use super::super::super::{WorkflowV2CallRecord, is_reusable_status, remediation_escalation};
use super::super::{PlannedRound, Residual, ResidualSeverity, RoundKind, round};
use super::{REFUSED_RED_GAP_ID, second_key};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::WorkflowV2ResultStore;
use crate::v2::verification::baseline_run_base::grouped_names;
use crate::v2::write::test_baseline_owner::{Ownership, ownership};
use crate::v2::write::test_baseline_owner_at::{parent_module_file_at, test_file_at};
use crate::v2::write::test_baseline_run_base::{Tree, cached, host_runnable};

/// The gap owed for `judge`, the latest verifier of the first-pass round
/// `own`, when the host refused it over red tests outside `own`'s scope.
pub(super) fn refused_red_tests(
    store: &WorkflowV2ResultStore,
    judge: &WorkflowV2CallRecord,
    own: &PlannedRound,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
) -> Option<Residual> {
    // A review round's claim is its refused unit's, not a gap list: it is
    // not retried here, and the gate weighs its refusal as before.
    if is_reusable_status(judge.status) || own.kind == RoundKind::Review {
        return None;
    }
    let views: Vec<&Value> = judge
        .result
        .data
        .get("outcomes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let mut tests: Vec<String> = views
        .iter()
        .filter_map(|view| view.pointer("/result/data/baseline_red_tests"))
        .flat_map(strings)
        .collect();
    tests.sort();
    tests.dedup();
    let mut commands: Vec<String> = views
        .iter()
        .filter_map(|view| {
            view.pointer("/result/commands_run")
                .and_then(Value::as_array)
        })
        .flatten()
        .filter(|command| command.get("status").and_then(Value::as_str) == Some("failed"))
        .filter_map(|command| command.get("command").and_then(Value::as_str))
        .map(str::trim)
        .filter(|command| host_runnable(command))
        .map(str::to_string)
        .collect();
    commands.sort();
    commands.dedup();
    let judged = remediation_escalation::judged_commit(&judge.result);
    let at = judged.as_deref();
    let tasks: Vec<String> = own.tasks.iter().cloned().collect();
    let granted: Vec<String> = own.files.iter().cloned().collect();
    let mut listed: Vec<String> = Vec::new();
    let mut files: BTreeSet<String> = BTreeSet::new();
    for test in &tests {
        let Some((command, file)) = commands
            .iter()
            .find_map(|command| test_file_at(root, at, command, test).map(|file| (command, file)))
        else {
            continue;
        };
        if ownership(Some(universe), &tasks, &granted, &file) == Ownership::Current {
            continue;
        }
        listed.push(test.clone());
        files.insert(file);
        files.extend(parent_module_file_at(root, at, command, test));
        // Every file the host's own judged run saw the test fail in.
        if let Some(run) = at.and_then(|commit| cached(store, Tree::Judged, commit, command)) {
            files.extend(run.failure_files.get(test).cloned().unwrap_or_default());
        }
    }
    if listed.is_empty() {
        return None;
    }
    Some(Residual {
        recorded_by: judge.call.id.clone(),
        id: format!("{REFUSED_RED_GAP_ID}@{}", judge.call.id),
        severity: ResidualSeverity::Medium,
        description: format!(
            "host refused `{}` over red tests outside its round's scope; make them pass: {}",
            judge.call.id,
            grouped_names(&listed)
        ),
        files: files.into_iter().collect(),
        unit_tasks: own.tasks.clone(),
        recorded_summary: judge.result.summary.clone(),
    })
}

/// The retry of `own` carrying `red`, or `red` with why it cannot be.
pub(super) fn retry_round(
    own: &PlannedRound,
    red: Residual,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
) -> Result<PlannedRound, (Residual, String)> {
    let mut tasks = own.tasks.clone();
    let mut unowned: BTreeSet<String> = BTreeSet::new();
    for file in &red.files {
        let declared_by = owners(universe, file, root);
        if declared_by.is_empty() {
            unowned.insert(file.clone());
        } else {
            tasks.extend(declared_by);
        }
    }
    let opened = expandable(universe, &tasks, &unowned, root);
    if let Some(closed) = unowned.difference(&opened).next() {
        let why = format!(
            "no task declares {closed}, and the host may not open it: its ownership is unprovable, it is a protected path, or a task forbids it"
        );
        return Err((red, why));
    }
    let files: BTreeSet<String> = own.files.union(&opened).cloned().collect();
    let kind = if files.is_empty() {
        RoundKind::Owned
    } else {
        RoundKind::Expansion
    };
    let mut residuals = own.residuals.clone();
    residuals.push(red);
    Ok(second_key(round(
        kind,
        tasks.into_iter().collect(),
        files,
        residuals,
        None,
        None,
    )))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
