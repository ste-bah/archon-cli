//! Issue-112: a path several tasks declare, deleted by one of them in a
//! verified landing.
//!
//! # The contradiction
//!
//! Issue-104 discharges an absence when the task whose single-task landing
//! deleted the path had it verified on a commit without it. "The task whose
//! landing deleted it" was taken as THE owner. It need not be: live on
//! wf-0ddadd81 the task universe declares `registry-migration-report.json`
//! for one task, whose fix kept and refreshed it, while another task's review
//! remediation -- granted the path because nothing else in its wave claimed
//! it -- deleted it as a stray, and that task's verifier accepted the tree
//! without it. Per path, the second task's verdict would have discharged the
//! first task's deliverable, and nothing would have said the two tasks
//! disagree; without a discharge, the open obligation re-dispatches the
//! first task's writes, which re-deliver the file and silently overturn the
//! verified deletion. Whichever ran last wins, either way.
//!
//! # The rule
//!
//! A path's DECLARERS are every task the task universe declares it for
//! (files expected to change, shared-append targets, deliverable contracts)
//! and every task a single-task landing manifest of this run declared it for
//! (the host's own write grants). Taking the newest landed deletion whose
//! deleting task's own later verification accepted a commit without the
//! path:
//!
//! - absent now, and EVERY other declarer's latest host-attributed
//!   verification after that deletion accepted a commit without it too: the
//!   absence is discharged -- all declarers agree (with one declarer, exactly
//!   Issue-104);
//! - absent now, and some declarer has no such verification: CONTESTED;
//! - present now, re-landed by another declarer after the deletion, and the
//!   deleting task's latest verification after that re-landing did not
//!   accept a commit holding it: CONTESTED as well -- a re-delivery does not
//!   overturn a verified deletion by being last.
//!
//! A contested path stays unresolved -- the final gate fails and names it,
//! so the run terminates and never claims the deliverable either way -- but
//! it is not a delivery obligation: cache admission does not re-dispatch its
//! declarers to re-deliver it (`reuse::eligible`), and a write that touches
//! it is told it is contested. It resolves when the declarers' own
//! verifications agree, or by an operator waiver. The task files are never
//! read for prose and never changed; only the host's records decide.

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::discharge::{Discharge, deletions, landed_at, task_ids, verified_state};
use super::{AuditReport, Verdict};
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::result_store::WorkflowV2ResultStore;
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, declared_covers, declared_path_form, declared_paths_of,
};
use crate::write_coordinator::{ManifestStatus, PatchManifest};

/// A declared path whose declarers' verified landings disagree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contest {
    pub declared_path: String,
    pub snapshot: String,
    /// `absent` or `present`: what the snapshot holds.
    pub state: String,
    /// The task whose landing deleted the path, the stage that landed it,
    /// and the accepted verification that judged the tree without it.
    pub deleted_by: String,
    pub deletion_stage: String,
    pub verification_call_id: String,
    /// Declarers with no accepted verification of the path as it now is.
    pub unconfirmed: Vec<String>,
}

impl Contest {
    /// One line naming the contradiction, for a gate's error.
    pub fn describe(&self) -> String {
        format!(
            "{} is contested: {} deleted it in {} (verified by {}), but {} declare{} it and no verification of {} accepted it {}",
            self.declared_path,
            self.deleted_by,
            self.deletion_stage,
            self.verification_call_id,
            self.unconfirmed.join(", "),
            if self.unconfirmed.len() == 1 { "s" } else { "" },
            if self.unconfirmed.len() == 1 {
                "it"
            } else {
                "them"
            },
            if self.state == "absent" {
                "without it"
            } else {
                "re-delivered"
            },
        )
    }
}

/// Every discharge and contest the host's records support for `report`.
pub fn judge(
    run_dir: &Path,
    report: &AuditReport,
    repository: &Path,
) -> (Vec<Discharge>, Vec<Contest>) {
    let store = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let Ok(records) = store.load_call_records() else {
        return (Vec::new(), Vec::new());
    };
    let universe = universe(run_dir);
    let manifests = manifests(run_dir, &store);
    let mut discharges = Vec::new();
    let mut contests = Vec::new();
    for record in &report.records {
        let path = record.declared_path.as_str();
        let absent = record.verdict == Verdict::Absent;
        if !absent && record.verdict != Verdict::ExistsAsDeclared {
            continue;
        }
        let mut found = deletions(run_dir, &store, path);
        found.sort_by(|left, right| right.at.cmp(&left.at));
        // The newest deletion its own task's verification stood behind.
        let Some((deletion, (verification, commit))) = found.iter().find_map(|deletion| {
            verified_state(
                &records,
                &deletion.task,
                &deletion.at,
                path,
                true,
                repository,
            )
            .map(|verified| (deletion, verified))
        }) else {
            continue;
        };
        let declarers = declarers(&universe, &manifests, path, repository);
        let contest = |state: &str, unconfirmed: Vec<String>| Contest {
            declared_path: path.to_string(),
            snapshot: report.snapshot.clone(),
            state: state.to_string(),
            deleted_by: deletion.task.clone(),
            deletion_stage: deletion.stage.clone(),
            verification_call_id: verification.clone(),
            unconfirmed,
        };
        if absent {
            let unconfirmed: Vec<String> = declarers
                .iter()
                .filter(|task| **task != deletion.task)
                .filter(|task| {
                    verified_state(&records, task, &deletion.at, path, true, repository).is_none()
                })
                .cloned()
                .collect();
            if unconfirmed.is_empty() {
                discharges.push(Discharge {
                    declared_path: path.to_string(),
                    snapshot: report.snapshot.clone(),
                    verification_call_id: verification.clone(),
                    base_commit: commit.clone(),
                });
            } else {
                contests.push(contest("absent", unconfirmed));
            }
            continue;
        }
        // Present: re-landed after the deletion by another declarer.
        let relanded = manifests
            .iter()
            .filter(|landed| landed.task != deletion.task && landed.at > deletion.at)
            .filter(|landed| landed.applied && landed.wrote.contains(path))
            .map(|landed| landed.at.as_str())
            .max();
        if let Some(at) = relanded
            && verified_state(&records, &deletion.task, at, path, false, repository).is_none()
        {
            contests.push(contest("present", vec![deletion.task.clone()]));
        }
    }
    (discharges, contests)
}

/// One landing manifest of this run a single task owns.
struct Landed {
    task: String,
    at: String,
    applied: bool,
    /// Applied or an idempotent no-op: a grant the host acted on.
    granted: bool,
    declared: BTreeSet<String>,
    wrote: BTreeSet<String>,
}

fn manifests(run_dir: &Path, store: &WorkflowV2ResultStore) -> Vec<Landed> {
    let Ok(stages) = std::fs::read_dir(run_dir.join("write-coordination").join("stages")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for stage in stages.flatten() {
        let Ok(entries) = std::fs::read_dir(stage.path().join("manifests")) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(manifest) = std::fs::read(entry.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PatchManifest>(&bytes).ok())
            else {
                continue;
            };
            let item = manifest.item_id.to_string();
            let Ok(Some(outcome)) = store.load_branch_outcome(&manifest.stage_id, &item) else {
                continue;
            };
            let owners = task_ids(
                outcome
                    .result
                    .as_ref()
                    .and_then(|result| result.data.get("canonical_task_ids")),
            );
            let Ok(Some(record)) = store.load_call_record(&manifest.stage_id) else {
                continue;
            };
            let [task] = owners.as_slice() else {
                continue;
            };
            found.push(Landed {
                task: task.clone(),
                at: landed_at(&record).to_string(),
                applied: manifest.status == ManifestStatus::Applied,
                granted: matches!(
                    manifest.status,
                    ManifestStatus::Applied | ManifestStatus::IdempotentNoop
                ),
                declared: manifest.declared_target_files.iter().cloned().collect(),
                wrote: manifest
                    .changed_files
                    .iter()
                    .chain(&manifest.created_files)
                    .cloned()
                    .collect(),
            });
        }
    }
    found
}

/// The tasks that declare `path`: in the task universe, and in this run's
/// single-task landing manifests.
fn declarers(
    universe: &Universe,
    manifests: &[Landed],
    path: &str,
    repository: &Path,
) -> BTreeSet<String> {
    let mut tasks: BTreeSet<String> = manifests
        .iter()
        .filter(|landed| landed.granted && landed.declared.contains(path))
        .map(|landed| landed.task.clone())
        .collect();
    let universe = match universe {
        Universe::Absent => return tasks,
        // Fail closed: who declares the path cannot be read, so nobody's
        // silence may discharge it.
        Universe::Unreadable => {
            tasks.insert(UNREADABLE_UNIVERSE.to_string());
            return tasks;
        }
        Universe::Present(universe) => universe,
    };
    for task in &universe.tasks {
        // An entry this host cannot read as one repository path (a template,
        // a traversal, a glob) may name this path: nothing may be concluded
        // from it (`DeclaredPathForm::Unusable`), so its task must confirm
        // like any declarer. An absolute path outside the repository never
        // names a repository path.
        let covers = declared_paths_of(task).iter().any(|entry| {
            match declared_path_form(entry, repository) {
                DeclaredPathForm::Repo(declared) => {
                    let scope = declared.trim_end_matches("/**");
                    scope.contains(['*', '?', '[', '{', '$']) || declared_covers(scope, path)
                }
                DeclaredPathForm::Unusable => true,
                DeclaredPathForm::Outside => false,
            }
        });
        if covers {
            tasks.insert(task.canonical_task_id.clone());
        }
    }
    tasks
}

/// The declarer named for a task universe the host could not read: no
/// verification ever confirms it.
pub const UNREADABLE_UNIVERSE: &str = "<unreadable task universe>";

/// The run's task universe as the host persisted it.
enum Universe {
    /// The run records none: no task declared anything beyond its landings.
    Absent,
    Unreadable,
    Present(WorkflowV2TaskUniverse),
}

fn universe(run_dir: &Path) -> Universe {
    let bytes = match std::fs::read(run_dir.join("v2/generated-metadata.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Universe::Absent,
        Err(_) => return Universe::Unreadable,
    };
    let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Universe::Unreadable;
    };
    match metadata.get("task_universe") {
        None | Some(serde_json::Value::Null) => Universe::Absent,
        Some(value) => {
            serde_json::from_value(value.clone()).map_or(Universe::Unreadable, Universe::Present)
        }
    }
}

#[cfg(test)]
#[path = "contest_tests.rs"]
mod tests;
