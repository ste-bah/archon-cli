//! Batch O: the host's plan for a review remediation pass.
//!
//! Before it dispatches anything, `remediateFindings` hands the host every
//! finding it was given on a checkpoint carrying [`REMEDIATION_PLAN_MARKER`]
//! (`options.findings`). The host answers on that checkpoint's view, under
//! [`REMEDIATION_PLAN_KEY`], one entry per finding in order:
//!
//! - `finding_id`: the host's id ([`finding_id_of`]), which every later
//!   rule reads;
//! - `task_ids` and `cross`: who fixes it. A finding naming universe tasks
//!   keeps them, widened (as one cross-task unit) to the owners of the files
//!   it names only when it names none of its own tasks' files. A finding naming no task
//!   -- at ANY severity -- is routed by content: the owners of the files it
//!   names, else the tasks whose own text names them, else the tasks whose
//!   `implements` it cites, else every task that may write (a PRD-level
//!   cross unit). Nothing is left unassigned;
//! - `grants`: files it names that no task declares and a round of those
//!   tasks may be granted ([`residual_paths::expandable`]). Grants come from
//!   this view only, never from a finding's own fields;
//! - `check`: whether it concerns a test or check, so its verifier must
//!   prove the check fails on a mutated copy ([`is_check_finding`]).
//!
//! and `task_scope`: every planned task's declared files, which join its
//! unit's write targets whatever the authored script listed. Computed at the
//! moment of asking from the universe and the tree, never persisted.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};

use super::residual_paths::{TaskTexts, expandable, named_files, owners};
use super::{WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Result};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::review_finding_ids::{canonical_text, finding_id_of};
use crate::v2::review_findings::task_ids_of;
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, declared_path_form, declared_paths_of,
};

pub use super::remediation_dispositions::is_check_finding;

/// The checkpoint option that asks the host for the plan.
pub const REMEDIATION_PLAN_MARKER: &str = "remediationPlan";
/// The checkpoint option carrying the findings to plan.
pub const REMEDIATION_PLAN_FINDINGS: &str = "findings";
/// Key of the plan in that checkpoint's view.
pub const REMEDIATION_PLAN_KEY: &str = "remediation_plan";

/// Whether `record` is a checkpoint asking for the plan.
pub fn asks_for_plan(record: &WorkflowV2CallRecord) -> bool {
    record.call.method == WorkflowV2HostMethod::Checkpoint
        && record.call.options.extra.get(REMEDIATION_PLAN_MARKER) == Some(&Value::Bool(true))
}

/// The findings a plan checkpoint asked about, as the script sent them.
pub fn planned_findings(record: &WorkflowV2CallRecord) -> Vec<Value> {
    record
        .call
        .options
        .extra
        .get(REMEDIATION_PLAN_FINDINGS)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `result` with the host's plan, for the view of the checkpoint that asked
/// for it; `None` for every other record. The key is the host's alone.
pub fn with_remediation_plan(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
    store: Option<&super::WorkflowV2ResultStore>,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Option<WorkflowV2Result> {
    let carried = result.data.get(REMEDIATION_PLAN_KEY).is_some();
    if !asks_for_plan(record) && !carried {
        return None;
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(REMEDIATION_PLAN_KEY);
    }
    if asks_for_plan(record) {
        let findings = planned_findings(record);
        // Files the findings name that no task declares are granted first,
        // through the run's recorded scope amendments; the plan is placed on
        // the amended universe.
        let amended = match (universe, root) {
            (Some(universe), Some(root)) => Some(amend::amended_universe(
                &findings,
                universe,
                root,
                store,
                Some(record.started_at.as_str()),
            )),
            _ => None,
        };
        let placed_on = amended
            .as_ref()
            .and_then(|a| a.universe.as_ref())
            .or(universe);
        let mut planned = plan(&findings, placed_on, root);
        if let Some(amended) = &amended {
            with_project_grants(&mut planned, &amended.project_grants);
        }
        viewed.data[REMEDIATION_PLAN_KEY] = planned;
    }
    Some(viewed)
}

/// The plan for `findings`; see the module doc. Without a universe or a
/// root the host can place nothing: every entry keeps its own tasks and no
/// grant, and the plan says so (`placed: false`), so nothing unplaced is
/// ever read as placed.
pub fn plan(
    findings: &[Value],
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Value {
    let (Some(universe), Some(root)) = (universe, root) else {
        let entries: Vec<Value> = findings
            .iter()
            .enumerate()
            .map(|(index, finding)| {
                json!({"index": index, "finding_id": finding_id_of(finding),
                    "task_ids": task_ids_of(finding), "cross": finding.get("attributable_to_task") == Some(&Value::Bool(false)),
                    "grants": [], "check": is_check_finding(finding), "basis": "unplaced"})
            })
            .collect();
        return json!({"source": "host", "placed": false, "findings": entries, "task_scope": {}});
    };
    let texts = TaskTexts::read(universe, root);
    let known: BTreeSet<&str> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.as_str())
        .collect();
    let mut scope_tasks: BTreeSet<String> = BTreeSet::new();
    let entries: Vec<Value> = findings
        .iter()
        .enumerate()
        .map(|(index, finding)| {
            let placed = place(finding, universe, root, &texts, &known);
            scope_tasks.extend(placed.tasks.iter().cloned());
            json!({
                "index": index,
                "finding_id": finding_id_of(finding),
                "task_ids": placed.tasks,
                "cross": placed.cross,
                "grants": placed.grants,
                "named_files": placed.named,
                "check": is_check_finding(finding),
                "basis": placed.basis,
            })
        })
        .collect();
    let task_scope: BTreeMap<String, Vec<String>> = scope_tasks
        .into_iter()
        .map(|task| {
            let files = task_scope_of(universe, &task, root);
            (task, files)
        })
        .collect();
    json!({"source": "host", "placed": true, "findings": entries, "task_scope": task_scope})
}

/// Each entry's project-data grants (`project_grants`: project-relative
/// path -> grantee tasks); a finding the plan placed on no task goes to its
/// grantees. The host stamps the grants on the grantees' branches at
/// dispatch, so they are never write targets of the script's.
fn with_project_grants(plan: &mut Value, grants: &[BTreeMap<String, BTreeSet<String>>]) {
    let Some(entries) = plan["findings"].as_array_mut() else {
        return;
    };
    for (entry, grants) in entries.iter_mut().zip(grants) {
        if grants.is_empty() {
            continue;
        }
        if entry["task_ids"].as_array().is_none_or(Vec::is_empty) {
            let tasks: BTreeSet<&String> = grants.values().flatten().collect();
            entry["task_ids"] = json!(tasks);
            entry["cross"] = json!(tasks.len() > 1);
        }
        entry["project_grants"] = json!(grants);
    }
}

struct Placed {
    tasks: Vec<String>,
    cross: bool,
    grants: Vec<String>,
    named: Vec<String>,
    basis: &'static str,
}

fn place(
    finding: &Value,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    texts: &TaskTexts,
    known: &BTreeSet<&str>,
) -> Placed {
    let text = finding_text(finding);
    let named = explicitly_named(&text, root);
    let owned_by: BTreeMap<&String, BTreeSet<String>> = named
        .iter()
        .map(|file| (file, owners(universe, file, root)))
        .collect();
    let unowned: BTreeSet<String> = owned_by
        .iter()
        .filter(|(_, by)| by.is_empty())
        .map(|(file, _)| (*file).clone())
        .collect();
    let own: BTreeSet<String> = task_ids_of(finding)
        .into_iter()
        .filter(|id| known.contains(id.as_str()))
        .collect();
    let opted_out = finding.get("attributable_to_task") == Some(&Value::Bool(false));
    let (tasks, cross, basis) = if own.is_empty() {
        let (tasks, spans, basis) =
            route_ownerless(universe, root, texts, &text, &owned_by, &unowned);
        let cross = spans || tasks.len() > 1;
        (tasks, cross, basis)
    } else {
        // The finding names files, and none is its own tasks': the fix lies
        // in other tasks' files, so the unit spans their owners. A finding
        // that names any file of its own tasks stays theirs.
        let names_own = owned_by.values().any(|by| !by.is_disjoint(&own));
        let others: BTreeSet<String> = if names_own {
            BTreeSet::new()
        } else {
            owned_by.values().flatten().cloned().collect()
        };
        let widened = !others.is_empty();
        let tasks: BTreeSet<String> = own.union(&others).cloned().collect();
        let basis = if widened {
            "named files of other tasks"
        } else {
            "finding"
        };
        // One unit for a finding of several tasks: per-task units would each
        // judge the same id, and one's "resolved" would close what the
        // other's verifier left open.
        let cross = opted_out || widened || tasks.len() > 1;
        (tasks, cross, basis)
    };
    let grants = expandable(universe, &tasks, &unowned, root);
    Placed {
        tasks: tasks.into_iter().collect(),
        cross,
        grants: grants.into_iter().collect(),
        named,
        basis,
    }
}

/// Who fixes a finding that names no universe task, by what it is about.
fn route_ownerless(
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    texts: &TaskTexts,
    text: &str,
    owned_by: &BTreeMap<&String, BTreeSet<String>>,
    unowned: &BTreeSet<String>,
) -> (BTreeSet<String>, bool, &'static str) {
    let owning: BTreeSet<String> = owned_by.values().flatten().cloned().collect();
    if !owning.is_empty() {
        return (owning, false, "owners of named files");
    }
    let naming: BTreeSet<String> = unowned
        .iter()
        .flat_map(|file| texts.naming(file, root))
        .collect();
    if !naming.is_empty() {
        return (naming, false, "tasks naming the named files");
    }
    let citing: BTreeSet<String> = universe
        .tasks
        .iter()
        .filter(|task| {
            task.implements
                .iter()
                .any(|req| !req.trim().is_empty() && cites(text, req.trim()))
        })
        .map(|task| task.canonical_task_id.clone())
        .collect();
    if !citing.is_empty() {
        return (citing, false, "tasks implementing cited requirements");
    }
    let writers: BTreeSet<String> = universe
        .tasks
        .iter()
        .filter(|task| !task_scope_of(universe, &task.canonical_task_id, root).is_empty())
        .map(|task| task.canonical_task_id.clone())
        .collect();
    (writers, true, "every task that may write")
}

/// Whether `text` cites `id` as a whole token.
fn cites(text: &str, id: &str) -> bool {
    text.match_indices(id).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + id.len()..].chars().next();
        let boundary = |c: Option<char>| {
            c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        };
        boundary(before) && boundary(after)
    })
}

/// The repository files `text` names by their own path or its last two
/// segments: a glob or a directory it mentions routes and grants nothing on
/// its own, so a finding about one file never opens a whole tree.
pub fn explicitly_named(text: &str, root: &Path) -> Vec<String> {
    named_files(text, root)
        .into_iter()
        .filter(|file| {
            let short = file
                .rmatch_indices('/')
                .nth(1)
                .map(|(at, _)| &file[at + 1..])
                .unwrap_or(file);
            text.contains(file.as_str()) || text.contains(short)
        })
        .collect()
}

/// Every string a finding carries, in canonical order: what its named files
/// and cited ids are read from.
pub fn finding_text(finding: &Value) -> String {
    let mut out = String::new();
    collect_strings(finding, &mut out);
    if out.is_empty() {
        out = canonical_text(finding);
    }
    out
}

fn collect_strings(value: &Value, out: &mut String) {
    match value {
        Value::String(text) => {
            out.push_str(text);
            out.push('\n');
        }
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            for key in keys {
                collect_strings(&object[key], out);
            }
        }
        _ => {}
    }
}

/// The literal repository files `task` declares (its own files, shared
/// targets and in-repository deliverables): directories and globs are
/// scope roots, never write targets, so they are left out.
pub fn task_scope_of(universe: &WorkflowV2TaskUniverse, task: &str, root: &Path) -> Vec<String> {
    let Some(entry) = universe
        .tasks
        .iter()
        .find(|candidate| candidate.canonical_task_id == task)
    else {
        return Vec::new();
    };
    let files: BTreeSet<String> = declared_paths_of(entry)
        .into_iter()
        .filter_map(|raw| match declared_path_form(&raw, root) {
            DeclaredPathForm::Repo(path) => Some(path),
            _ => None,
        })
        .filter(|path| !path.is_empty() && !path.ends_with('/') && !path.contains('*'))
        .filter(|path| !root.join(path).is_dir())
        .collect();
    files.into_iter().collect()
}

#[path = "remediation_plan_amend.rs"]
mod amend;
pub use amend::landed_files_by_task;

#[cfg(test)]
#[path = "remediation_plan_tests.rs"]
mod tests;
