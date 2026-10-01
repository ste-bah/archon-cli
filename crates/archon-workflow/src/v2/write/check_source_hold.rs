//! PLAN-11: a write branch's change to a pinned acceptance-check source is
//! held out of its landing, and recorded as a re-author request.
//!
//! The frozen acceptance chain (contract, skeleton, locks, pin store) rejects
//! any landing that touches it (Batch O, I11). The sources a frozen check
//! RUNS -- the integration test it names, the module files that test loads,
//! the unit test function a filter selects, the script it executes -- sat in
//! grantable scope, and only prompt prose forbade weakening them. Here each
//! such change is taken out of the worktree before the scope grant and every
//! ownership gate read it (a whole file is restored, or removed when it was
//! created; a unit test function is spliced back to its pinned text, or out,
//! the rest of its file kept), so the branch's other work lands as it would
//! have. The proposed bytes are recorded under the run
//! (`check_source_requests`), and the next acceptance round has the judge
//! decide them (`check_source_settle`): accepted, the host applies and
//! re-pins them; refused, they stay out and the refusal is shown to the next
//! proposer here.
//!
//! This is also how a test the task must CREATE lands: a target absent at
//! freeze is pinned absent (or its name watched), so creating it is a
//! pinned-source change like any other -- held, judged, applied, pinned --
//! and never an unpinned edit.
//!
//! A change the host cannot take back out of the worktree -- a pinned test
//! function deleted from a file the branch also changed, which has nowhere
//! to be spliced back -- is returned as `unheld`, and the branch is rejected
//! for it like a forbidden path: it must never land unjudged.

use super::*;

use archon_write_plan::WritePlan;

use crate::check_source_drift::{SourceChange, landing_changes};
use crate::check_source_pins::{PinStore, load_for_run};
use crate::check_source_requests::{
    CHECK_SOURCE_HELD_GAP_PREFIX, CHECK_SOURCE_PINS_UNAVAILABLE_GAP_PREFIX, NewRequest,
    ORIGIN_LANDING, SourceChangeRequest, last_refusal, record,
};
use crate::check_source_resolve::Roots;
use crate::check_source_rust::{item_text, splice_item};
use crate::task_set_contract::content_digest;

/// What the hold did for one branch.
#[derive(Debug, Default)]
pub(super) struct Hold {
    /// Each held change's recorded request.
    pub(super) held: Vec<SourceChangeRequest>,
    /// Changes that could not be taken out: the branch must not land.
    pub(super) unheld: Vec<String>,
    /// Why the pins could not be read at all, when they could not.
    pub(super) unavailable: Option<String>,
    pub(super) refusals: Vec<String>,
}

/// Hold every pinned check-source change in `plan`'s worktree out of the
/// landing, record it, and strike held files from the envelope. Runs after
/// the scope grant and its drops, only for a branch that is about to land,
/// and only on paths the grant opened (`opened`) or the project inputs the
/// landing applies. Pins that exist but cannot be read, or a worktree whose
/// changes cannot be listed, are `unavailable`: the branch is refused (fail
/// closed), never landed unread.
pub(super) fn hold_check_source_changes(
    plan: &WritePlan,
    opened: &dyn Fn(&str) -> bool,
    run_root: &Path,
    (call_id, branch_id): (&str, &str),
    task_ids: &[String],
    result: &mut WorkflowV2Result,
) -> Hold {
    let mut hold = Hold::default();
    let policy = match landing_policy(run_root) {
        Ok(Some(policy)) => policy,
        Ok(None) => return hold,
        Err(error) => {
            hold.unavailable = Some(error);
            return hold;
        }
    };
    let roots = Roots {
        repository: &plan.canonical_root,
        project: &policy.project,
    };
    let (store, pins) = match load_for_run(run_root, &policy.project, &policy.task_root, &roots) {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return hold,
        Err(error) => {
            hold.unavailable = Some(error);
            return hold;
        }
    };
    let changed = match crate::write_coordinator::patch_manifest::workspace_changed_paths(
        &plan.isolated_root,
    ) {
        Ok(changed) => changed
            .into_iter()
            .filter(|path| opened(path))
            .collect::<Vec<_>>(),
        Err(error) => {
            hold.unavailable = Some(format!(
                "the worktree's changes could not be listed: {error}"
            ));
            return hold;
        }
    };
    let inputs = crate::write_coordinator::project_inputs::ProjectInputPolicy::for_run(run_root);
    let project_input = |path: &str| inputs.as_ref().is_some_and(|policy| policy.covers(path));
    let view = crate::check_source_drift::LandingView {
        worktree: &plan.isolated_root,
        project: &policy.project,
        project_input: &project_input,
    };
    let changes = landing_changes(&pins, &changed, &view);
    let site = HoldSite {
        plan,
        project: &policy.project,
        store: &store,
        run_root,
        ids: (call_id, branch_id),
        task_ids,
    };
    let mut by_file: std::collections::BTreeMap<String, Vec<&SourceChange>> = Default::default();
    for change in &changes {
        if change.item.is_some() {
            by_file.entry(change.path.clone()).or_default().push(change);
            continue;
        }
        match hold_file(&site, change) {
            Ok(request) => {
                result
                    .files_changed
                    .retain(|file| !same_path(plan, &file.path, &request.path));
                hold.held.push(request);
            }
            Err(why) => hold.unheld.push(format!("{} ({why})", change.label())),
        }
    }
    for (path, items) in by_file {
        match hold_items(&site, &path, &items) {
            Ok(requests) => hold.held.extend(requests),
            Err(why) => hold.unheld.push(format!("{path} ({why})")),
        }
    }
    for request in &hold.held {
        if let Some(refusal) = last_refusal(run_root, &request.path) {
            hold.refusals.push(refusal);
        }
    }
    hold.refusals.sort();
    hold.refusals.dedup();
    hold
}

/// The landing policy that names the run's task set; `None` only for a run
/// that records none. A run whose launch record names a task set, but whose
/// policy cannot be read, is an error: its landings are refused, never
/// landed unpinned.
fn landing_policy(
    run_root: &Path,
) -> Result<Option<crate::write_coordinator::project_inputs::ProjectInputPolicy>, String> {
    if let Some(policy) =
        crate::write_coordinator::project_inputs::ProjectInputPolicy::for_landing(run_root)
    {
        return Ok(Some(policy));
    }
    let path = run_root.join("v2/generated-metadata.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{} could not be read: {error}", path.display())),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is malformed: {error}", path.display()))?;
    let names_task_set = value
        .pointer("/observer_snapshot/canonical_task_root_identity")
        .is_some()
        || value
            .pointer("/observer_snapshot/native_execution/policy")
            .is_some();
    if names_task_set {
        return Err(format!(
            "{} names the run's task set, but no landing policy can be read from it, so the frozen checks' sources cannot be recognized",
            path.display()
        ));
    }
    Ok(None)
}

impl Hold {
    /// The rejection the branch gets instead of landing, if any: pins that
    /// could not be read (operational: the normal retry re-runs the branch
    /// once they can be), else `forbidden` paths or changes that could not be
    /// held (semantic, like a forbidden path).
    pub(super) fn rejection(
        &self,
        branch_id: &str,
        task_ids: &[String],
        forbidden: &[String],
    ) -> Option<(WorkflowV2Result, &'static str)> {
        if let Some(why) = &self.unavailable {
            return Some((
                pins_unavailable_result(branch_id, task_ids, why),
                "check_source_pins_unavailable",
            ));
        }
        let refused: Vec<String> = forbidden.iter().chain(&self.unheld).cloned().collect();
        (!refused.is_empty()).then(|| {
            (
                super::forbidden_paths::forbidden_rejection_result(branch_id, task_ids, &refused),
                "forbidden_path_changed",
            )
        })
    }
}

fn pins_unavailable_result(item_id: &str, task_ids: &[String], why: &str) -> WorkflowV2Result {
    let failure_kind = BranchFailureKind::Execution;
    let (status, evidence_kind, severity) = branch_validation_failure_fields(&failure_kind);
    let summary = format!(
        "write item '{item_id}' was not landed: the frozen checks' pinned sources could not be read ({why}), so no change to a check source could be recognized; the branch is re-run once they can be"
    );
    let mut result = WorkflowV2Result {
        status,
        summary: truncate_for_result(&summary, 2_000),
        ..WorkflowV2Result::default()
    };
    result
        .evidence
        .push(WorkflowV2Evidence::new(evidence_kind, summary.clone()));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{CHECK_SOURCE_PINS_UNAVAILABLE_GAP_PREFIX}{}",
            sanitize_v2_path_segment(item_id)
        ),
        description: truncate_for_result(&summary, 1_000),
        severity: Some(severity.to_string()),
    });
    result.data = serde_json::json!({
        "branch_id": item_id,
        "item_id": item_id,
        "canonical_task_ids": task_ids,
        "branch_error_from_runtime": true,
        "failure_kind": failure_kind,
        "error": truncate_for_result(&summary, 2_000),
        "check_source_pins_unavailable": why,
        "patch_landed": false,
    });
    result
}

struct HoldSite<'a> {
    plan: &'a WritePlan,
    project: &'a Path,
    store: &'a PinStore,
    run_root: &'a Path,
    ids: (&'a str, &'a str),
    task_ids: &'a [String],
}

fn same_path(plan: &WritePlan, reported: &str, path: &str) -> bool {
    let reported = reported.trim_start_matches("./");
    reported == path
        || Path::new(reported) == plan.canonical_root.join(path)
        || Path::new(reported) == plan.isolated_root.join(path)
}

/// A whole pinned file: record the proposal, then restore or remove it.
fn hold_file(site: &HoldSite, change: &SourceChange) -> Result<SourceChangeRequest, String> {
    let worktree = site.plan.isolated_root.join(&change.path);
    let proposed = match std::fs::symlink_metadata(&worktree) {
        Ok(_) => Some(
            crate::check_source_pins::bytes_at(&worktree, None)
                .ok_or("the worktree copy could not be read")?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("the worktree copy could not be read: {error}")),
    };
    let request = record(
        site.run_root,
        new_request(
            change,
            site.ids,
            site.task_ids,
            proposed.as_deref(),
            None,
            None,
        ),
    )?;
    if change.root == crate::check_source_resolve::SourceRoot::Project {
        // A project-input copy goes back to the project's own bytes, so the
        // landing's input capture finds nothing to apply.
        return match std::fs::read(site.project.join(&change.path)) {
            Ok(bytes) => std::fs::write(&worktree, bytes).map(|()| request),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::remove_file(&worktree).map(|()| request)
            }
            Err(error) => Err(error),
        }
        .map_err(|error| format!("the worktree copy could not be put back: {error}"));
    }
    let restored = crate::write_coordinator::whitespace_only::restore_or_remove(
        &site.plan.isolated_root,
        std::slice::from_ref(&change.path),
    );
    let back = std::fs::read(&worktree)
        .ok()
        .map(|bytes| content_digest(&bytes));
    if restored.is_empty() && back.is_some() {
        return Err("the worktree copy could not be restored".into());
    }
    Ok(request)
}

/// Every pinned item of one file: each spliced back to its pinned text (or
/// out, or -- a manifest entry -- back in), the rest of the file kept, and
/// each recorded against the file as it lands.
fn hold_items(
    site: &HoldSite,
    path: &str,
    changes: &[&SourceChange],
) -> Result<Vec<SourceChangeRequest>, String> {
    let worktree = site.plan.isolated_root.join(path);
    let original = std::fs::read_to_string(&worktree).map_err(|error| error.to_string())?;
    let mut landed = original.clone();
    let mut proposals = Vec::new();
    for change in changes {
        let key = change.item.as_deref().expect("item changes only");
        let proposed = item_text(&original, key);
        let pinned = pinned_item(site, change, key);
        if change.pinned.is_some() && pinned.is_none() {
            return Err(format!(
                "the pinned text of {key} is not retained, so it cannot be put back"
            ));
        }
        let reinsertable = key.starts_with("toml:") || key == crate::check_source_rust::CFG_KEY;
        if proposed.is_none() && pinned.is_some() && !reinsertable {
            return Err(format!(
                "pinned {key} was deleted from a file the branch also changed; restore it, and propose any test change on its own"
            ));
        }
        landed = splice_item(&landed, key, pinned.as_deref())
            .ok_or_else(|| format!("{key} could not be spliced back"))?;
        proposals.push((change, proposed));
    }
    let landed_digest = content_digest(landed.as_bytes());
    let mut requests = Vec::new();
    for (change, proposed) in proposals {
        requests.push(record(
            site.run_root,
            new_request(
                change,
                site.ids,
                site.task_ids,
                proposed.as_deref().map(str::as_bytes),
                Some(original.as_bytes()),
                Some(landed_digest.clone()),
            ),
        )?);
    }
    std::fs::write(&worktree, landed).map_err(|error| error.to_string())?;
    Ok(requests)
}

fn new_request<'a>(
    change: &'a SourceChange,
    (call_id, branch_id): (&'a str, &'a str),
    task_ids: &[String],
    proposed: Option<&'a [u8]>,
    proposed_file: Option<&'a [u8]>,
    landed_file_digest: Option<String>,
) -> NewRequest<'a> {
    NewRequest {
        origin: ORIGIN_LANDING,
        check_ids: change.check_ids.clone(),
        root: change.root,
        path: &change.path,
        item: change.item.as_deref(),
        was_pinned: change.was_pinned,
        pinned_digest: change.pinned.clone(),
        proposed,
        proposed_file,
        landed_file_digest,
        call_id,
        branch_id,
        task_ids: task_ids.to_vec(),
    }
}

/// The pinned text of item `key`: from the pin's blob, else from the
/// canonical file when its item still hashes to the pin.
fn pinned_item(site: &HoldSite, change: &SourceChange, key: &str) -> Option<String> {
    let digest = change.pinned.as_ref()?;
    if let Some(bytes) = site.store.blobs.get(digest) {
        return String::from_utf8(bytes).ok();
    }
    let canonical = std::fs::read_to_string(site.plan.canonical_root.join(&change.path)).ok()?;
    item_text(&canonical, key).filter(|item| content_digest(item.as_bytes()) == *digest)
}

/// Record what the hold did as a review gap, an evidence line and result
/// data, whatever the branch's status. Status and summary are untouched.
pub(super) fn report_check_source_holds(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    hold: &Hold,
) {
    if hold.held.is_empty() && hold.unheld.is_empty() && hold.unavailable.is_none() {
        return;
    }
    let held = hold
        .held
        .iter()
        .map(|request| {
            format!(
                "{} for check(s) {} as request {}",
                request.label(),
                request
                    .check_ids
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                request.request_id
            )
        })
        .collect::<Vec<_>>();
    // What happened to the rest is the branch's verdict, read after every
    // gate: a branch refused later lands nothing, held change included.
    let rest = if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        "the rest of the branch's work goes on to land"
    } else {
        "the branch itself was not accepted, so nothing of it lands and the held change is never applied from this attempt"
    };
    let mut description = format!(
        "write item '{branch_id}' changed {} source(s) a frozen acceptance check runs; each change was held out of this landing ({rest}) and recorded for the acceptance judge, which applies and re-pins it only if the check still fails whenever its criterion is false and its branch landed: {}. Do not re-apply a held change by any other route; it lands through the judge or not at all.",
        hold.held.len(),
        held.join("; ")
    );
    if !hold.refusals.is_empty() {
        description.push_str(&format!(
            " Earlier proposals: {}.",
            hold.refusals.join("; ")
        ));
    }
    if !hold.unheld.is_empty() {
        description.push_str(&format!(
            " These could not be taken out of the worktree, so the branch was rejected: {}.",
            hold.unheld.join("; ")
        ));
    }
    if let Some(why) = &hold.unavailable {
        description.push_str(&format!(
            " The check-source pins could not be read ({why}); no change to a check source could be recognized in this landing."
        ));
    }
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{CHECK_SOURCE_HELD_GAP_PREFIX}{}",
            sanitize_v2_path_segment(branch_id)
        ),
        description,
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!(
            "pinned acceptance-check source changes held for the judge ({}): {}",
            hold.held.len(),
            held.join("; ")
        ),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "check_source_held".to_string(),
            serde_json::json!(
                hold.held
                    .iter()
                    .map(|request| serde_json::json!({
                        "path": request.path,
                        "item": request.item,
                        "check_ids": request.check_ids,
                        "request_id": request.request_id,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
    }
}

#[cfg(test)]
#[path = "check_source_hold_tests.rs"]
mod tests;
