//! Issue-104: an absence the task's own verifier accepted is not a finding.
//!
//! The declared-target floor (Issue-92) makes every file a task lists under
//! "Files Expected to Change" a declared path, and the audit turns a declared
//! path that does not exist into an `absent -> deliver` obligation. A task can
//! list a path it must NOT leave behind (a report listed with status
//! `absent`, whose notes say it must never exist at the repository root).
//! Live, the obligation then re-dispatched an earlier wave, which re-created
//! the file a verified review remediation had deleted.
//!
//! The independent verifier read the whole task and is the authority on its
//! contract. So an absence obligation on a declared path is DISCHARGED when
//! the owning task's latest host-recorded verification -- a task verify, or a
//! review-remediation verify that ran an agent -- was accepted and judged a
//! tree in which the path was absent (its base commit does not hold it). The
//! discharge names the verifying call. It binds one snapshot and one absent
//! verdict: a later write that re-creates the path is judged as it exists.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{AuditReport, Verdict};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::result::WorkflowV2Status;
use crate::v2::result_store::{WorkflowV2CallRecord, WorkflowV2ResultStore};
use crate::v2::script::{AuthoredCallRole, authored_call_role, task_outcomes};

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

/// Whether `record` is a verification of `task` an agent performed.
fn verifies(record: &WorkflowV2CallRecord, task: &str) -> bool {
    match authored_call_role(&record.call) {
        AuthoredCallRole::RemediationVerify {
            task: key, agent, ..
        } => {
            let spans = record
                .call
                .options
                .extra
                .get("remediationContract")
                .and_then(|contract| contract.get("taskIds"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(task)));
            agent && (key == task || spans)
        }
        AuthoredCallRole::TaskVerify => task_outcomes(record, false).0.contains_key(task),
        _ => false,
    }
}

fn finished(record: &WorkflowV2CallRecord) -> &str {
    if record.finished_at.is_empty() {
        &record.started_at
    } else {
        &record.finished_at
    }
}

/// The commit a verification's branches judged, from the host's own
/// baseline record (`write::test_baseline`).
fn judged_commit(v2_root: &Path, call_id: &str) -> Option<String> {
    let entries = std::fs::read_dir(v2_root.join("baseline-tests").join(call_id)).ok()?;
    entries.flatten().find_map(|entry| {
        let bytes = std::fs::read(entry.path()).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value["base_commit"]
            .as_str()
            .map(str::trim)
            .filter(|commit| !commit.is_empty())
            .map(str::to_string)
    })
}

fn present_at(repository: &Path, commit: &str, path: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["cat-file", "-e", &format!("{commit}:{path}")])
        .output()
        .map_or(true, |out| out.status.success())
}

/// Whether a task's declaration names `path` (relative, or absolute under
/// the repository).
fn declares(universe: &WorkflowV2TaskUniverse, task: &str, path: &str, repository: &Path) -> bool {
    let root = repository.display().to_string();
    universe
        .tasks
        .iter()
        .filter(|entry| entry.canonical_task_id == task)
        .flat_map(|entry| entry.files_expected_to_change.iter())
        .filter_map(|entry| crate::v2::script::declared_path(entry))
        .any(|declared| {
            let declared = declared.trim_start_matches("./");
            declared == path
                || declared
                    .strip_prefix(&root)
                    .map(|rest| rest.trim_start_matches('/'))
                    == Some(path)
        })
}

/// Discharges the host's records support for `report`'s absent paths.
pub fn verified_absences(
    run_dir: &Path,
    report: &AuditReport,
    repository: &Path,
) -> Vec<Discharge> {
    let v2_root = run_dir.join("v2");
    let Some(universe) = std::fs::read(v2_root.join("generated-metadata.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|meta| {
            serde_json::from_value::<WorkflowV2TaskUniverse>(meta["task_universe"].clone()).ok()
        })
    else {
        return Vec::new();
    };
    let Ok(records) = WorkflowV2ResultStore::new(&v2_root).load_call_records() else {
        return Vec::new();
    };
    let mut discharges = Vec::new();
    for record in report
        .records
        .iter()
        .filter(|r| r.verdict == Verdict::Absent)
    {
        let path = record.declared_path.as_str();
        for task in universe.tasks.iter().map(|t| t.canonical_task_id.as_str()) {
            if !declares(&universe, task, path, repository) {
                continue;
            }
            let latest = records
                .iter()
                .filter(|r| r.invalidated_by.is_none() && verifies(r, task))
                .max_by(|left, right| finished(left).cmp(finished(right)));
            let Some(latest) = latest.filter(|r| r.status == WorkflowV2Status::Accepted) else {
                continue;
            };
            let Some(commit) = judged_commit(&v2_root, &latest.call.id) else {
                continue;
            };
            if !present_at(repository, &commit, path) {
                discharges.push(Discharge {
                    declared_path: path.to_string(),
                    snapshot: report.snapshot.clone(),
                    verification_call_id: latest.call.id.clone(),
                    base_commit: commit,
                });
                break;
            }
        }
    }
    discharges
}

#[cfg(test)]
#[path = "discharge_tests.rs"]
mod tests;
