//! Issue-104: an absence the owning task created, deleted and had verified
//! is not a finding.
//!
//! The declared-target floor (Issue-92) makes every file a task lists under
//! "Files Expected to Change" a declared path, and the audit turns a declared
//! path that does not exist into an `absent -> deliver` obligation. A task can
//! list a path it must NOT leave behind; its status reads `absent` exactly as
//! a file still to be created does, so the declaration cannot tell them
//! apart. Live, the obligation re-dispatched an earlier wave, which
//! re-created a file a verified review remediation had deleted.
//!
//! Only what the host itself recorded decides. An absence obligation is
//! DISCHARGED when all of these hold:
//! * the OWNING task deleted the path: an applied manifest of this run, from
//!   a stage whose only task is that one, records the path deleted -- the
//!   owner is that task, and a path no task created-then-deleted (a file
//!   still to be delivered) is never discharged;
//! * the owning task's latest verification finished after that stage -- a
//!   task verify or an agent review verify, attributed to the task by the
//!   host (never by the agent's own report) -- was accepted for that task;
//!   a newer rejected one blocks;
//! * the commit that verification judged, recorded on its own record when it
//!   ran ([`JUDGED_COMMIT_KEY`]), does not hold the path. Records written
//!   before that stamp existed discharge nothing.
//!
//! A discharge binds one snapshot and that snapshot's absent verdict: a later
//! write that re-creates the path is judged as it exists.
//!
//! Issue-112: "the owning task" is not the only declarer. A path several
//! tasks declare is discharged only when every one of them confirmed the
//! absence the same way; otherwise it is contested (`super::contest`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::AuditReport;
use crate::v2::result::{WorkflowV2Result, WorkflowV2Status};
use crate::v2::result_store::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use crate::v2::script::{AuthoredCallRole, authored_call_role, task_outcomes};
use crate::write_coordinator::{ManifestStatus, PatchManifest};

/// Where a verification branch's result records the commit it judged.
pub const JUDGED_COMMIT_KEY: &str = "judged_commit";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Discharge {
    pub declared_path: String,
    pub snapshot: String,
    /// The accepted verification that judged the path absent.
    pub verification_call_id: String,
    /// The commit that verification judged.
    pub base_commit: String,
}

/// Record the commit a verification branch judged on its own result.
pub fn stamp_judged_commit(result: &mut WorkflowV2Result, commit: Option<&str>) {
    let Some(commit) = commit.map(str::trim).filter(|commit| !commit.is_empty()) else {
        return;
    };
    if result.data.is_null() {
        result.data = Value::Object(Default::default());
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(JUDGED_COMMIT_KEY.into(), Value::String(commit.into()));
    }
}

pub(super) fn finished(record: &WorkflowV2CallRecord) -> &str {
    if record.finished_at.is_empty() {
        &record.started_at
    } else {
        &record.finished_at
    }
}

/// When a landing stage's execution finished: the execution its record
/// restates when a later session re-saved it (Issue-111), else the record's
/// own finish. A replay is not a new landing.
pub(super) fn landed_at(record: &WorkflowV2CallRecord) -> &str {
    record
        .answered_by
        .as_ref()
        .map_or_else(|| finished(record), |origin| origin.finished_at.as_str())
}

/// A landed deletion of `path`: the only task of the stage, the stage, and
/// when it finished.
pub(super) struct Deletion {
    pub(super) task: String,
    pub(super) stage: String,
    pub(super) at: String,
}

pub(super) fn task_ids(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Every applied manifest of this run that records `path` deleted, owned by
/// the single task its branch outcome names.
pub(super) fn deletions(
    run_dir: &Path,
    store: &WorkflowV2ResultStore,
    path: &str,
) -> Vec<Deletion> {
    let Ok(stages) = std::fs::read_dir(run_dir.join("write-coordination").join("stages")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for stage in stages.flatten() {
        let Ok(manifests) = std::fs::read_dir(stage.path().join("manifests")) else {
            continue;
        };
        for entry in manifests.flatten() {
            let Some(manifest) = std::fs::read(entry.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PatchManifest>(&bytes).ok())
            else {
                continue;
            };
            let deleted = manifest.deleted_files.iter().any(|p| p.as_str() == path)
                || manifest.post_hashes.get(path).map(String::as_str) == Some("deleted");
            if manifest.status != ManifestStatus::Applied || !deleted {
                continue;
            }
            let item_id = manifest.item_id.to_string();
            let Ok(Some(outcome)) = store.load_branch_outcome(&manifest.stage_id, &item_id) else {
                continue;
            };
            let owners = task_ids(
                outcome
                    .result
                    .as_ref()
                    .and_then(|r| r.data.get("canonical_task_ids")),
            );
            let Ok(Some(record)) = store.load_call_record(&manifest.stage_id) else {
                continue;
            };
            if let [task] = owners.as_slice() {
                found.push(Deletion {
                    task: task.clone(),
                    stage: manifest.stage_id.clone(),
                    at: landed_at(&record).to_string(),
                });
            }
        }
    }
    found
}

fn is_verification(record: &WorkflowV2CallRecord) -> bool {
    matches!(
        authored_call_role(&record.call),
        AuthoredCallRole::TaskVerify | AuthoredCallRole::RemediationVerify { agent: true, .. }
    )
}

/// The commit a verification judged for `task`, from the branch views the
/// host attributes to that task. Every such view must agree.
fn judged_commit(record: &WorkflowV2CallRecord, task: &str) -> Option<String> {
    let views = record.result.data.get("outcomes")?.as_array()?;
    let single_graph_item = record.source_task_graph.as_ref().is_some_and(|graph| {
        graph.items.len() == 1 && graph.items[0].canonical_task_ids.iter().any(|t| t == task)
    });
    let dispatched: Vec<&str> = record
        .dispatched_items
        .iter()
        .filter(|item| item.canonical_task_ids.iter().any(|t| t == task))
        .map(|item| item.item_id.as_str())
        .collect();
    // Every attributed view must carry the stamp: one without it ran before
    // the stamp existed, and its commit is unknown.
    let commits: Option<Vec<&str>> = views
        .iter()
        .filter(|view| {
            let id = view
                .get("item_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            single_graph_item || dispatched.contains(&id)
        })
        .map(|view| {
            view.pointer(&format!("/result/data/{JUDGED_COMMIT_KEY}"))?
                .as_str()
        })
        .collect();
    let commits = commits?;
    let first = *commits.first()?;
    commits
        .iter()
        .all(|c| *c == first)
        .then(|| first.to_string())
}

fn git_ok(repository: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Whether `commit` is a real commit that does not hold `path`. An unreadable
/// commit holds everything: only a proven absence discharges.
pub(super) fn absent_at(repository: &Path, commit: &str, path: &str) -> bool {
    git_ok(
        repository,
        &["cat-file", "-e", &format!("{commit}^{{commit}}")],
    ) && !git_ok(repository, &["cat-file", "-e", &format!("{commit}:{path}")])
}

/// `task`'s latest host-attributed verification finished after `after`,
/// when it accepted `task` on a commit whose state of `path` is `absent`
/// (lacking it) or not (holding it): that verification and the commit. A
/// newer rejected one, an unstamped one or a commit git cannot read answers
/// `None`.
pub(super) fn verified_state(
    records: &[WorkflowV2CallRecord],
    task: &str,
    after: &str,
    path: &str,
    absent: bool,
    repository: &Path,
) -> Option<(String, String)> {
    let (record, outcome) = records
        .iter()
        .filter(|r| r.invalidated_by.is_none() && is_verification(r))
        .filter(|r| finished(r) > after)
        .filter_map(|r| {
            let (tasks, agent_attributed) = task_outcomes(r, false);
            let outcome = tasks.get(task).copied()?;
            (!agent_attributed).then_some((r, outcome))
        })
        .max_by(|left, right| finished(left.0).cmp(finished(right.0)))?;
    if outcome.status != WorkflowV2Status::Accepted || outcome.not_reviewed {
        return None;
    }
    let commit = judged_commit(record, task)?;
    let holds = git_ok(
        repository,
        &["cat-file", "-e", &format!("{commit}^{{commit}}")],
    ) && git_ok(repository, &["cat-file", "-e", &format!("{commit}:{path}")]);
    let matches = if absent {
        absent_at(repository, &commit, path)
    } else {
        holds
    };
    matches.then(|| (record.call.id.clone(), commit))
}

/// Discharges the host's records support for `report`'s absent paths
/// (Issue-104), each confirmed by every task that declares the path
/// (Issue-112, [`super::contest::judge`]).
pub fn verified_absences(
    run_dir: &Path,
    report: &AuditReport,
    repository: &Path,
) -> Vec<Discharge> {
    super::contest::judge(run_dir, report, repository).0
}

#[cfg(test)]
#[path = "discharge_tests.rs"]
mod tests;
