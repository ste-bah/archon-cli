//! One bounded cross-owner round for a remediation unit whose verifier
//! refuted the fix over files another task owns (Issue-107).
//!
//! # The dead end this ends
//!
//! `remediateFindings` re-asked every round with the findings it built once
//! per unit, and a refuting verifier only ended the round. When the refusal
//! was "the change this needs is in files another task owns", nothing could
//! act on it: a single task's forbidden list stays as written, cross-task
//! units come only from findings a reducer marked `attributable_to_task:
//! false`, and the owning task's own fixer is handed only its own findings.
//! After `maxRounds` the unit ended `unverified`, which holds the run. Seen
//! live: one task's fix added a fail-closed gate and its verifier
//! refuted it because two must-pass tests in a file a later task owns call
//! the gated writer; round 2 was asked the same question.
//!
//! # What the host concludes, and from what
//!
//! For a remediation VERIFY call that did not accept, the host reads the
//! verdict's `blocker` evidence for repository paths -- the evidence
//! `source` and path-shaped tokens in its summary -- and maps each through
//! the task universe's declared files (`files_expected_to_change` and
//! shared-append targets, read as the verifier's ownership stamp reads them:
//! [`declared_path_form`] against the repository root, [`declared_covers`]
//! for a declared directory). A path the unit's own tasks declare is theirs
//! already; a path another task declares names that task as an owner. The
//! agent's prose about WHICH task owns a file is never read: only its paths,
//! and only the universe says who owns them.
//!
//! The plan is attached to the call's result as the script sees it, under
//! [`REMEDIATION_ESCALATION_KEY`], on every path that answers the call --
//! run, replayed, drifted or history -- because it is a pure function of the
//! recorded verdict and the universe. It is never persisted, so no stored
//! record changes and no reuse identity moves.
//!
//! # What it can and cannot buy
//!
//! A plan grants the ability to WRITE, never a verdict. The prelude spends it
//! on exactly one extra round per unit, whose write names the owners beside
//! the unit's tasks -- so the existing cross-task plumbing applies unchanged:
//! each task's declared floor (`task_declared_targets`), and forbidden
//! patterns lifted only where they lie wholly inside a declared path of one
//! of the item's tasks (`ForbiddenPaths::without_within`). Nothing outside
//! the involved tasks' declared paths is ever lifted. Its verifier judges the
//! original findings over every involved task, and the host stamps each of
//! those tasks' must-pass baselines on it; the round's verdict is the unit's
//! outcome. A plan is never made for an escalated round, so there is no
//! second one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};

use super::{
    WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result,
    is_reusable_status, remediation_contract,
};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, declared_covers, declared_path_form,
};

/// Key of the plan in a remediation verify's result data, as the script
/// reads it (the data keys are spread onto the envelope's top level).
pub const REMEDIATION_ESCALATION_KEY: &str = "remediation_escalation";
/// The contract key that marks the escalated round itself.
pub const ESCALATION_CONTRACT_KEY: &str = "escalation";

/// Most blocker paths one plan widens a round by.
const MAX_PATHS: usize = 12;
/// Most blocker evidence entries quoted to the escalated round.
const MAX_EVIDENCE: usize = 6;
/// Characters kept of each quoted blocker summary.
const EVIDENCE_CHARS: usize = 600;
/// Characters kept of the refusing verdict's summary.
const REFUTATION_CHARS: usize = 1_200;

/// Whether `call` is the escalated round of a remediation unit.
pub fn is_escalated_remediation(call: &WorkflowV2HostCall) -> bool {
    remediation_contract(call)
        .is_some_and(|contract| contract.get(ESCALATION_CONTRACT_KEY).is_some())
}

/// The cross-owner plan for a refused remediation verdict, or `None` when
/// the call is no such verdict or no blocker path maps to another task.
pub fn escalation_plan(
    call: &WorkflowV2HostCall,
    result: &WorkflowV2Result,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Option<Value> {
    let universe = universe?;
    let contract = remediation_contract(call)?;
    if contract.get("stage").and_then(Value::as_str) != Some("verify")
        || call.method == WorkflowV2HostMethod::Checkpoint
        || is_escalated_remediation(call)
        || is_reusable_status(result.status)
    {
        return None;
    }
    let unit_tasks = unit_task_ids(contract);
    let evidence = blocker_evidence(result);
    let mut owned_by: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in evidence
        .iter()
        .flat_map(|(summary, source)| candidate_paths(summary, source.as_deref(), repository_root))
    {
        let owners = declaring_tasks(universe, &path, repository_root);
        if owners.is_empty() || owners.iter().any(|task| unit_tasks.contains(task)) {
            continue;
        }
        if owned_by.len() < MAX_PATHS || owned_by.contains_key(&path) {
            owned_by.entry(path).or_default().extend(owners);
        }
    }
    if owned_by.is_empty() {
        return None;
    }
    let owners: BTreeSet<&String> = owned_by.values().flatten().collect();
    Some(json!({
        "source": "host",
        "unit_task_ids": unit_tasks,
        "owner_task_ids": owners,
        "target_files": owned_by.keys().collect::<Vec<_>>(),
        "owned_by": owned_by.iter().map(|(path, owners)| json!({
            "path": path, "owner_task_ids": owners,
        })).collect::<Vec<_>>(),
        "refutation": clip(&result.summary, REFUTATION_CHARS),
        "blocker_evidence": evidence.iter().take(MAX_EVIDENCE).map(|(summary, source)| json!({
            "summary": clip(summary, EVIDENCE_CHARS), "source": source,
        })).collect::<Vec<_>>(),
    }))
}

/// Whether `record` is the refused verdict of its unit's LAST regular round
/// and carries a host plan: the answer the escalated round is bought with,
/// replayed as history on a resume (`resume_drift`).
pub fn buys_escalation(
    record: &super::WorkflowV2CallRecord,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> bool {
    let Some(contract) = remediation_contract(&record.call) else {
        return false;
    };
    let round = contract.get("round").and_then(Value::as_u64);
    round.is_some()
        && round == contract.get("maxRounds").and_then(Value::as_u64)
        && escalation_plan(&record.call, &record.result, universe, repository_root).is_some()
}

/// `result` with the plan in its data, for the script's view of it; `None`
/// when the view is the result as it was. The key is the host's alone: one
/// already in the data (nothing the host writes puts it there) is dropped,
/// so no answer can hand the script a plan of its own.
pub fn with_escalation_plan(
    call: &WorkflowV2HostCall,
    result: &WorkflowV2Result,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Option<WorkflowV2Result> {
    let plan = escalation_plan(call, result, universe, repository_root);
    let carried = result.data.get(REMEDIATION_ESCALATION_KEY).is_some();
    if plan.is_none() && !carried {
        return None;
    }
    let mut viewed = result.clone();
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(REMEDIATION_ESCALATION_KEY);
    }
    if let Some(plan) = plan {
        if !viewed.data.is_object() {
            viewed.data = json!({});
        }
        viewed.data[REMEDIATION_ESCALATION_KEY] = plan;
    }
    Some(viewed)
}

/// The tasks a remediation unit speaks for: the contract's `taskIds` for a
/// cross-task unit, else its `taskId`.
fn unit_task_ids(contract: &Value) -> BTreeSet<String> {
    let listed: BTreeSet<String> = contract
        .get("taskIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    if !listed.is_empty() {
        return listed;
    }
    contract
        .get("taskId")
        .and_then(Value::as_str)
        .map(|id| BTreeSet::from([id.trim().to_string()]))
        .unwrap_or_default()
}

/// Every `blocker` evidence entry the verdict carries -- its own lifted
/// evidence and each branch's -- as (summary, source), deduplicated, in
/// the order met.
fn blocker_evidence(result: &WorkflowV2Result) -> Vec<(String, Option<String>)> {
    let mut found: Vec<(String, Option<String>)> = result
        .evidence
        .iter()
        .filter(|evidence| evidence.kind == WorkflowV2EvidenceKind::Blocker)
        .map(|evidence| (evidence.summary.clone(), evidence.source.clone()))
        .collect();
    collect_blockers(&result.data, 0, &mut found);
    let mut seen = BTreeSet::new();
    found.retain(|entry| seen.insert(entry.clone()));
    found
}

fn collect_blockers(value: &Value, depth: usize, found: &mut Vec<(String, Option<String>)>) {
    if depth > 6 {
        return;
    }
    match value {
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_blockers(item, depth + 1, found)),
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("blocker")
                && let Some(summary) = object.get("summary").and_then(Value::as_str)
            {
                let source = object
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                found.push((summary.to_string(), source));
            }
            object
                .values()
                .for_each(|item| collect_blockers(item, depth + 1, found));
        }
        _ => {}
    }
}

/// Repository-relative paths one blocker names: its structured `source`
/// when that is a clean repository path, and only when it is not, every
/// path-shaped token of its summary (a `/` in it, a `:line` suffix
/// dropped). Anything that is not a clean repository path is ignored.
fn candidate_paths(summary: &str, source: Option<&str>, root: Option<&Path>) -> Vec<String> {
    if let Some(path) = source.and_then(|source| repository_path(source, root)) {
        return vec![path];
    }
    summary
        .split(|c: char| c.is_whitespace() || "()[]{},;'\"`<>".contains(c))
        .filter_map(|token| repository_path(token, root))
        .collect()
}

fn repository_path(token: &str, root: Option<&Path>) -> Option<String> {
    let mut token = token
        .trim()
        .trim_end_matches(['.', ',', ';', ':', '!', '?']);
    // `path:12` or `path:12:4` names a line in `path`.
    while let Some((head, tail)) = token.rsplit_once(':') {
        if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        token = head;
    }
    if !token.contains('/') || token.contains("://") {
        return None;
    }
    let relative = match root {
        Some(root) => match declared_path_form(token, root) {
            DeclaredPathForm::Repo(path) => path,
            _ => return None,
        },
        None if token.starts_with('/') => return None,
        None => token.trim_start_matches("./").to_string(),
    };
    let clean = !relative.is_empty()
        && !relative.starts_with('/')
        && !relative
            .chars()
            .any(|c| c.is_whitespace() || "*?[".contains(c))
        && !relative
            .split('/')
            .any(|segment| segment == ".." || segment.is_empty());
    clean.then_some(relative)
}

/// Every task that declares `path` as a writable FILE: the exact file, or a
/// file that exists in the repository under a directory the task declares.
/// A directory is never a blocker path, so an owner's scope is never opened
/// wholesale, and a path the root cannot confirm as a file under a declared
/// directory names no owner.
fn declaring_tasks(
    universe: &WorkflowV2TaskUniverse,
    path: &str,
    root: Option<&Path>,
) -> BTreeSet<String> {
    universe
        .tasks
        .iter()
        .filter(|task| {
            task.files_expected_to_change
                .iter()
                .chain(&task.shared_append_target_files)
                .filter_map(|entry| super::declared_path(entry))
                .filter_map(|declared| {
                    let directory = declared.ends_with('/') || declared.ends_with("/**");
                    let relative = match root {
                        Some(root) => match declared_path_form(&declared, root) {
                            DeclaredPathForm::Repo(path) => path,
                            _ => return None,
                        },
                        None => declared.trim_start_matches("./").to_string(),
                    };
                    Some((relative.trim_end_matches("/**").to_string(), directory))
                })
                .any(|(declared, directory)| {
                    let in_tree =
                        |test: fn(&Path) -> bool| root.is_some_and(|root| test(&root.join(path)));
                    let file = !directory && declared == path && !in_tree(Path::is_dir);
                    file || (declared != path
                        && declared_covers(&declared, path)
                        && in_tree(Path::is_file))
                })
        })
        .map(|task| task.canonical_task_id.clone())
        .collect()
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}...", text.chars().take(limit).collect::<String>())
}

#[path = "remediation_escalation_dispatch.rs"]
mod dispatch;
pub use dispatch::{escalation_refusal, refused_escalation_result, script_view};

#[cfg(test)]
#[path = "remediation_escalation_tests.rs"]
mod tests;
