//! A remediation round that landed nothing after a refusal, on a tree the
//! run changed since that refusal (Issue-111).
//!
//! # The dead end this ends
//!
//! A fix that lands no patch is never verified: the verifier would judge the
//! code the reviewers already judged (`landedNothing` in the prelude). That
//! holds only while the tree is the one the last verdict saw. Live on
//! wf-0ddadd81: a unit's two rounds were refused over must-pass tests in a
//! file another task owns; that task's own remediation then landed the fix
//! there; the unit's escalated round found the red baseline green and landed
//! nothing -- and the refusal it was bought with held the run, because no
//! verifier was ever asked about the tree as it had become.
//!
//! # What the host concludes, and from what
//!
//! For a remediation FIX record that the host marked as landing nothing
//! (`patch_landed` false, none true), accepted, with an idempotent manifest,
//! the host finds the refused verdict of the same unit this session answered
//! last before that round -- the one the script's `lastRefusal` holds -- and
//! compares two commits it recorded itself:
//!
//! - the commit that verdict judged (`judged_commit`, stamped on every
//!   read-only verification branch when it ran), and
//! - the commit the fix was dispatched on (its manifest's baseline commit's
//!   first parent: the canonical HEAD the host sealed the worktree on).
//!
//! Every landing of THIS run between them (the host's own commits, ordered by
//! git, `branch_cache::landing::run_landings_between`) is read for the paths
//! it touched. The tree MOVED when one of them touched a path the unit or its
//! blocker names: a path the fix was granted (its manifest's declared
//! targets, which carry every involved task's floor and the escalated blocker
//! files), a blocker path the refusal named, or the escalation's blocker
//! paths. Agent prose is never read for any of it.
//!
//! Both commits are fixed facts of the records, so the answer is the same on
//! every resume: a later landing never reopens a round whose fix was already
//! dispatched past it. Anything the host cannot prove -- no refusal, no
//! judged commit, no manifest, a history git cannot order -- is no plan, and
//! the refusal stands as before.
//!
//! # What it can and cannot buy
//!
//! The plan rides on the fix's view under [`REMEDIATION_REVERIFY_KEY`] and is
//! never persisted. It buys ONE read-only verifier for that round, named
//! after the fix's ordinal so no later call's id moves, whose contract names
//! the fix and the refusal ([`REVERIFY_CONTRACT_KEY`]); the host answers such
//! a call only when its own plan for this session's fix of that round agrees
//! ([`reverify_refusal`]). The verdict is the round's, like any verifier's:
//! it grants no write and resolves nothing the terminal rule does not
//! already require -- an accepted fix and an accepted agent verifier, same
//! round, fix first.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use super::super::resume_drift::remediation_unit;
use super::super::resume_verdict::{is_remediation_fix, remediation_round_key};
use super::super::{
    WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, is_reusable_status, remediation_contract,
    remediation_contract_string,
};
use super::{blocker_evidence, candidate_paths, dispatch};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::branch_cache::landing::{first_parent, run_landings_between};
use crate::v2::verification::path_ownership::{DeclaredPathForm, declared_path_form};
use crate::write_coordinator::{ManifestStatus, PatchManifest};

/// Key of the plan in a no-patch fix's result data, as the script reads it.
pub const REMEDIATION_REVERIFY_KEY: &str = "remediation_reverify";
/// The contract key that marks the re-verification call itself.
pub const REVERIFY_CONTRACT_KEY: &str = "reverify";

/// Most moved paths one plan names.
const MAX_PATHS: usize = 24;

/// The host's re-verification plan for `record`, a remediation fix that
/// landed nothing: `None` unless the run's own landings since the refusal
/// before its round touched a path the unit or its blockers name.
pub fn reverify_plan(
    record: &WorkflowV2CallRecord,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Option<Value> {
    let root = repository_root?;
    let call = &record.call;
    if !is_remediation_fix(call)
        || call.method == WorkflowV2HostMethod::Checkpoint
        || call.write_mode.is_none()
        || !is_reusable_status(record.status)
        || !landed_nothing(&record.result.data)
    {
        return None;
    }
    let (unit, round) = remediation_unit(call)?;
    let records = store.load_call_records().ok()?;
    let refusal = dispatch::last_refusal(&records, store, &unit, round)?;
    let judged = judged_commit(&refusal.result)?;
    let manifest = fix_manifest(store, record)?;
    let dispatched = first_parent(root, &manifest.baseline_commit).ok()?;
    let landings = run_landings_between(root, &record.run_id, &judged, &dispatched).ok()?;
    let relevant = relevant_paths(record, &manifest, &refusal.result, universe, root);
    let mut moved: BTreeSet<&str> = BTreeSet::new();
    let mut by: Vec<Value> = Vec::new();
    for landing in &landings {
        let touched: Vec<&str> = landing
            .paths
            .iter()
            .map(String::as_str)
            .filter(|path| relevant.contains(*path))
            .collect();
        if touched.is_empty() {
            continue;
        }
        moved.extend(touched.iter().copied());
        by.push(json!({"commit": landing.commit, "stage": landing.stage, "paths": touched}));
    }
    if moved.is_empty() {
        return None;
    }
    Some(json!({
        "source": "host",
        "fix_call_id": call.id,
        "refusal_call_id": refusal.call.id,
        "judged_commit": judged,
        "dispatch_commit": dispatched,
        "moved_paths": moved.iter().take(MAX_PATHS).collect::<Vec<_>>(),
        "landings": by,
    }))
}

/// `result` with the plan in its data, for the script's view of `record`;
/// `None` when the view is `result` as it was. The key is the host's alone:
/// one already in the data is dropped.
pub fn with_reverify_plan(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Option<WorkflowV2Result> {
    let plan = reverify_plan(record, store, universe, repository_root);
    let carried = result.data.get(REMEDIATION_REVERIFY_KEY).is_some();
    if plan.is_none() && !carried {
        return None;
    }
    let mut viewed = result.clone();
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(REMEDIATION_REVERIFY_KEY);
    }
    if let Some(plan) = plan {
        if !viewed.data.is_object() {
            viewed.data = json!({});
        }
        viewed.data[REMEDIATION_REVERIFY_KEY] = plan;
    }
    Some(viewed)
}

/// Why the re-verification call `execution` may not be answered, or `None`
/// when it is no such call or is exactly what the host's plan for this
/// session's fix of its round allows.
pub fn reverify_refusal(
    execution: &WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Option<String> {
    let call = &execution.call;
    let claimed = remediation_contract(call)?.get(REVERIFY_CONTRACT_KEY)?;
    let why = (|| {
        if remediation_contract_string(call, "stage") != Some("verify")
            || call.write_mode.is_some()
            || call.method == WorkflowV2HostMethod::Checkpoint
        {
            return Some("it is not a read-only verifier".to_string());
        }
        let Some(key) = remediation_round_key(call) else {
            return Some("its contract names no unit and round".to_string());
        };
        let records = match store.load_call_records() {
            Ok(records) => records,
            Err(error) => return Some(format!("the call records are unreadable: {error}")),
        };
        let Some(fix) = records
            .iter()
            .filter(|record| {
                is_remediation_fix(&record.call)
                    && store.in_session(&record.call.id)
                    && remediation_round_key(&record.call).as_deref() == Some(key.as_str())
            })
            .max_by_key(|record| dispatch::finished(record))
        else {
            return Some("no fix of its round was answered in this session".to_string());
        };
        let Some(plan) = reverify_plan(fix, store, universe, repository_root) else {
            return Some(format!(
                "the host has no re-verification plan for `{}`",
                fix.call.id
            ));
        };
        let agrees = |field: &str, planned: &str| claimed.get(field) == Some(&plan[planned]);
        if !agrees("fixCallId", "fix_call_id") || !agrees("refusalCallId", "refusal_call_id") {
            return Some(format!(
                "its contract does not match the plan for `{}` (refusal `{}`)",
                fix.call.id,
                plan["refusal_call_id"].as_str().unwrap_or_default()
            ));
        }
        None
    })()?;
    Some(format!("re-verification `{}` refused: {why}", call.id))
}

/// What the script is handed for a refused re-verification: no verdict.
pub fn refused_reverify_result(reason: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: reason.to_string(),
        data: json!({ "reverify_refused": reason }),
        ..WorkflowV2Result::default()
    }
}

/// The host's typed marker says nothing landed: `patch_landed` false
/// somewhere and true nowhere (the prelude's `landedNothing`, read the same
/// way round -- absent means "landed", never "nothing").
pub fn landed_nothing(data: &Value) -> bool {
    fn walk(value: &Value, depth: usize, seen: &mut (bool, bool)) {
        if depth > 8 {
            return;
        }
        match value {
            Value::Object(object) => {
                match object.get("patch_landed") {
                    Some(Value::Bool(true)) => seen.0 = true,
                    Some(Value::Bool(false)) => seen.1 = true,
                    _ => {}
                }
                object.values().for_each(|item| walk(item, depth + 1, seen));
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, depth + 1, seen)),
            _ => {}
        }
    }
    let mut seen = (false, false);
    walk(data, 0, &mut seen);
    !seen.0 && seen.1
}

/// The commit a refused verdict judged: every branch view carries the stamp
/// and they agree. A view without it ran before the stamp existed, and what
/// it judged is unknown.
pub(crate) fn judged_commit(result: &WorkflowV2Result) -> Option<String> {
    let views = result.data.get("outcomes")?.as_array()?;
    let commits: Option<BTreeSet<&str>> = views
        .iter()
        .map(|view| {
            view.pointer("/result/data/judged_commit")?
                .as_str()
                .map(str::trim)
                .filter(|commit| !commit.is_empty())
        })
        .collect();
    let commits = commits?;
    (commits.len() == 1).then(|| commits.into_iter().next().unwrap_or_default().to_string())
}

/// The fix's manifest: under its own stage, else under the recorded
/// execution this session replayed it from. Idempotent only: a fix whose
/// manifest records any change is no fix that landed nothing.
fn fix_manifest(
    store: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
) -> Option<PatchManifest> {
    let run_root = store.root().parent()?;
    let mut stages = vec![record.call.id.clone()];
    if let Some(source) =
        remediation_round_key(&record.call).and_then(|key| store.fix_replayed_from(&key))
    {
        stages.push(source);
    }
    if let Some(origin) = &record.answered_by {
        stages.push(origin.call_id.clone());
    }
    for stage in stages {
        let dir = run_root
            .join("write-coordination")
            .join("stages")
            .join(&stage)
            .join("manifests");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        let manifests: Vec<PatchManifest> = paths
            .iter()
            .filter_map(|path| serde_json::from_slice(&std::fs::read(path).ok()?).ok())
            .collect();
        let Some(first) = manifests.first() else {
            continue;
        };
        let idempotent = manifests.iter().all(|manifest| {
            manifest.status == ManifestStatus::IdempotentNoop
                && manifest.changed_files.is_empty()
                && manifest.created_files.is_empty()
                && manifest.deleted_files.is_empty()
                && manifest.baseline_commit == first.baseline_commit
        });
        return idempotent.then(|| first.clone());
    }
    None
}

/// The paths a landing must touch, exactly, for the tree to have moved
/// under the unit: the files the fix was granted and the escalation's
/// blocker files -- less every shared-append file, which any task's append
/// touches -- and each file the refusal's blocker evidence named that a task
/// of the universe declares as a file. Nothing a path covers counts: a
/// directory token names no file, and a landing elsewhere under it is no
/// change to what the refusal judged.
fn relevant_paths(
    record: &WorkflowV2CallRecord,
    manifest: &PatchManifest,
    refusal: &WorkflowV2Result,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: &Path,
) -> BTreeSet<String> {
    let repo_form = |entry: &String| {
        super::super::declared_path(entry).and_then(|path| match declared_path_form(&path, root) {
            DeclaredPathForm::Repo(path) => Some(path),
            _ => None,
        })
    };
    let tasks = universe.into_iter().flat_map(|universe| &universe.tasks);
    let shared: BTreeSet<String> = tasks
        .clone()
        .flat_map(|task| &task.shared_append_target_files)
        .filter_map(repo_form)
        .collect();
    let declared_files: BTreeSet<String> = tasks
        .flat_map(|task| {
            task.files_expected_to_change
                .iter()
                .chain(&task.shared_append_target_files)
        })
        .filter_map(repo_form)
        .collect();
    let named: BTreeSet<String> = blocker_evidence(refusal)
        .into_iter()
        .flat_map(|(summary, source)| candidate_paths(&summary, source.as_deref(), Some(root)))
        .filter(|path| declared_files.contains(path))
        .collect();
    let mut paths: BTreeSet<String> = manifest.declared_target_files.iter().cloned().collect();
    if let Some(blockers) = remediation_contract(&record.call)
        .and_then(|contract| contract.pointer("/escalation/blockerPaths"))
        .and_then(Value::as_array)
    {
        paths.extend(
            blockers
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );
    }
    paths.retain(|path| !path.trim().is_empty() && !shared.contains(path));
    paths.extend(named);
    paths
}

#[cfg(test)]
#[path = "remediation_escalation_reverify_tests.rs"]
mod tests;
