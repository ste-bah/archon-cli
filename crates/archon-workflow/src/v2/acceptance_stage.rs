//! The authored run's acceptance stage: its records and its task mapping.
//!
//! Obs-32: a v3 authored run reported "complete" while the task set's frozen
//! acceptance checks failed, because those checks ran only in a post-terminal
//! observe-only observer. The stage defined here runs INSIDE the authored
//! script as its final stage — `await acceptance()` in the v3 dialect, host
//! call `acceptance-contract-run` — and the authored lifecycle's terminal
//! status depends on what its final round recorded.
//!
//! This module owns the durable shape: one record per round attempt under
//! `<run>/v2/acceptance/<round>/attempt-<n>.json`, append-only, plus the
//! mapping from an acceptance check to the tasks that implement it. The host
//! that executes checks and the finalizer that reads the last record both go
//! through here, so neither can disagree with the other about what a failing
//! round looks like.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::{WorkflowError, WorkflowResult};

/// The local tool name the v3 primitive asks the host for.
pub const ACCEPTANCE_STAGE_TOOL: &str = "acceptance-contract-run";
/// Call-id prefix of every acceptance round; the round number follows it.
pub const ACCEPTANCE_STAGE_CALL_PREFIX: &str = "acceptance-contract-run-";
/// Records directory, relative to the run directory.
pub const ACCEPTANCE_RECORDS_DIR: &str = "v2/acceptance";
pub const ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION: u32 = 1;
/// The round count a script without its own asks for. Recorded on each
/// round, never a stop: the loop's budget follows progress
/// ([`progress::decide`]), not a count (A2).
pub const ACCEPTANCE_MAX_ROUNDS: u32 = 3;
/// A repair of checks whose frozen judgment was not `accepted`.
pub const REPAIR_TRIGGER_NOT_ACCEPTED: &str = "not_accepted";
/// A repair of checks that crashed in their own code when the round ran them
/// (`acceptance_check_crash`).
pub const REPAIR_TRIGGER_SCRIPT_DEFECT: &str = "script_defect";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceCheckStatus {
    Passed,
    Failed,
    /// The check could not be evaluated. Counted as failing: an unevaluated
    /// check is not a passed one.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCheckRecordV1 {
    pub check_id: String,
    pub criterion: String,
    pub kind: String,
    pub status: AcceptanceCheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operational_error: Option<String>,
    /// Tasks whose `implements` list names this check; empty for a set-level
    /// check no task implements.
    #[serde(default)]
    pub owning_tasks: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout_tail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr_tail: String,
    /// The run landing this check regressed at, when the host could show
    /// one (`acceptance_regression`): remediation goes to its tasks too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regressed_by: Option<super::acceptance_regression::AcceptanceRegressionV1>,
    /// The frozen check itself is defective — its judge verdict is not
    /// `accepted`, or it crashed in its own code — so no implementation can
    /// make it pass. Always failing and never owned — a contract defect is repaired by re-authoring the check,
    /// not by sending implementing tasks to chase a check that cannot run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub contract_defect: bool,
    /// The repository files the failure implicates and who can write them
    /// (`acceptance_routing`): remediation goes to those writers too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<super::acceptance_routing::AcceptanceRoutingV1>,
    /// Batch J: for a failed check no landing was shown to break, what the
    /// regression search established (it never held in the run, or why it
    /// stopped): the note its remediation is sent with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regression_search: Option<super::acceptance_regression::RegressionSearchV1>,
    /// Batch J: the rule that leaves this failed check with no unit able to
    /// fix it (`acceptance_routing::mark_blocked`). Such a check is never
    /// sent to a remediation round; it is raised as a HIGH operational
    /// finding instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

impl AcceptanceCheckRecordV1 {
    pub fn failing(&self) -> bool {
        self.status != AcceptanceCheckStatus::Passed
    }

    /// It RAN and FAILED, and is not the contract's to repair: the only
    /// kind of failing check a task is sent to fix.
    pub fn ran_and_failed(&self) -> bool {
        self.status == AcceptanceCheckStatus::Failed && !self.contract_defect
    }

    /// Whether any task is named to fix it: an owner, the tasks of the
    /// landing that broke it, or a writer of a file it implicates.
    pub fn routed(&self) -> bool {
        !self.owning_tasks.is_empty()
            || (self.regressed_by.as_ref()).is_some_and(|regression| !regression.tasks.is_empty())
            || (self.routing.as_ref())
                .is_some_and(super::acceptance_routing::AcceptanceRoutingV1::routes)
    }
}

/// One in-round repair of frozen checks that no task could fix: the host
/// re-authored and re-judged exactly `check_ids` and, on success, republished
/// the contract chain — before the round ran its checks (the judge had not
/// accepted them), or after they crashed in their own code this round, in
/// which case the repaired checks ran again in the same round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceContractRepairV1 {
    pub check_ids: Vec<String>,
    /// [`REPAIR_TRIGGER_NOT_ACCEPTED`] or [`REPAIR_TRIGGER_SCRIPT_DEFECT`];
    /// empty in records written before the trigger was recorded.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trigger: String,
    pub repaired: bool,
    /// The freeze event the republished pin carries; empty when not repaired.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub freeze_event_id: String,
    /// Why the repair did not produce accepted checks; empty when repaired.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub failure: String,
    /// What the repair reported on the way, e.g. why a re-authored check
    /// could not be executed before it was published.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

/// How and where the checks ran; recorded so a reader can tell a hermetic
/// scratch observation from a direct run in the live checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceExecutionRecordV1 {
    /// `scratch` (the `[workflow.acceptance_execution]` policy) or `direct`.
    pub mode: String,
    pub repository: String,
    pub project: String,
    pub task_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    #[serde(default)]
    pub dirty_worktree: bool,
    pub environment: String,
    pub timeout_secs: u64,
    pub config_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceRoundRecordV1 {
    pub schema_version: u32,
    pub run_id: String,
    pub call_id: String,
    pub round: u32,
    pub attempt: u32,
    pub max_rounds: u32,
    pub contract_present: bool,
    /// The ids this round was asked to run; empty means every check.
    #[serde(default)]
    pub requested_check_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<AcceptanceExecutionRecordV1>,
    #[serde(default)]
    pub checks: Vec<AcceptanceCheckRecordV1>,
    /// Errors that prevented the stage from evaluating at all (no contract
    /// root, unreadable contract, ...). Any entry makes the round failing.
    #[serde(default)]
    pub operational_errors: Vec<String>,
    /// Repairs of non-accepted frozen checks attempted before the checks ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contract_repairs: Vec<AcceptanceContractRepairV1>,
    /// Whether the script's loop ends here: a clean round, nothing left that
    /// a task or the host could act on, or no progress for
    /// [`progress::ACCEPTANCE_STALL_LIMIT`] consecutive rounds.
    pub final_round: bool,
}

impl AcceptanceRoundRecordV1 {
    pub fn failing_checks(&self) -> Vec<&AcceptanceCheckRecordV1> {
        self.checks.iter().filter(|check| check.failing()).collect()
    }

    pub fn failing_check_ids(&self) -> Vec<String> {
        self.failing_checks()
            .into_iter()
            .map(|check| check.check_id.clone())
            .collect()
    }

    /// Failing checks that are contract defects (see `contract_defect`).
    pub fn contract_defect_ids(&self) -> Vec<String> {
        self.checks
            .iter()
            .filter(|check| check.contract_defect)
            .map(|check| check.check_id.clone())
            .collect()
    }

    pub fn unowned_failing_check_ids(&self) -> Vec<String> {
        self.failing_checks()
            .into_iter()
            .filter(|check| check.owning_tasks.is_empty())
            .map(|check| check.check_id.clone())
            .collect()
    }

    pub fn passed_check_ids(&self) -> Vec<String> {
        self.checks
            .iter()
            .filter(|check| !check.failing())
            .map(|check| check.check_id.clone())
            .collect()
    }

    /// The round blocks completion: a failing check, a stage that could not
    /// evaluate, no contract at all, or a contract of which no check ran.
    /// Nothing passes vacuously (A8): the stage records a missing contract
    /// or an empty run as an operational error, and a record without one
    /// still blocks here.
    pub fn blocks_completion(&self) -> bool {
        !self.operational_errors.is_empty()
            || !self.contract_present
            || self.checks.is_empty()
            || self.checks.iter().any(|check| check.failing())
    }

    /// Whether the round leaves anything the loop can act on: a FAILED
    /// check some task is named to fix (its owners, the landing that broke
    /// it, the writers of its files, or the tasks the host reassigned it
    /// to), or a failure the host repairs itself before the next round runs
    /// -- a check in `Error` (the host's environment, Issue-128: rebuilt and
    /// re-run, never a task's) or a contract defect (re-authored by the
    /// host). A failed check still `blocked` after re-routing has no task
    /// in the universe at all.
    pub fn has_remediable_failures(&self) -> bool {
        self.failing_checks().iter().any(|check| {
            (check.ran_and_failed() && check.blocked.is_none() && check.routed())
                || check.status == AcceptanceCheckStatus::Error
                || check.contract_defect
        })
    }

    /// Failed checks a task is sent to fix: they ran, failed, are not the
    /// contract's, and name at least one owning task.
    pub fn task_remediable_check_ids(&self) -> Vec<String> {
        (self.failing_checks().into_iter())
            .filter(|check| check.ran_and_failed() && !check.owning_tasks.is_empty())
            .map(|check| check.check_id.clone())
            .collect()
    }

    /// Failed checks no unit can fix, each with its blocking rule.
    pub fn blocked_checks(&self) -> Vec<(String, String)> {
        self.checks
            .iter()
            .filter_map(|check| Some((check.check_id.clone(), check.blocked.clone()?)))
            .collect()
    }
}

#[path = "acceptance_coverage.rs"]
pub mod coverage;
#[path = "acceptance_progress.rs"]
pub mod progress;

/// Tasks whose declared `implements` list names `check_id`, sorted.
pub fn owning_tasks(universe: Option<&WorkflowV2TaskUniverse>, check_id: &str) -> Vec<String> {
    universe
        .map(|universe| {
            universe
                .tasks
                .iter()
                .filter(|task| task.implements.iter().any(|id| id == check_id))
                .map(|task| task.canonical_task_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .unwrap_or_default()
}

pub fn round_dir(run_dir: &Path, round: u32) -> PathBuf {
    run_dir
        .join(ACCEPTANCE_RECORDS_DIR)
        .join(format!("round-{round:02}"))
}

pub fn attempt_file_name(attempt: u32) -> String {
    format!("attempt-{attempt:02}.json")
}

/// The attempt number the next record of `round` takes: one past the highest
/// already on disk, a quarantined one included. Records are never
/// overwritten and a number is never reused.
pub fn next_attempt(run_dir: &Path, round: u32) -> u32 {
    let dir = round_dir(run_dir, round);
    (highest_attempt(&dir).max(progress::highest_quarantined_attempt(&dir)))
        .map_or(1, |attempt| attempt + 1)
}

fn highest_attempt(dir: &Path) -> Option<u32> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            name.strip_prefix("attempt-")?
                .strip_suffix(".json")?
                .parse::<u32>()
                .ok()
        })
        .max()
}

fn highest_round(run_dir: &Path) -> Option<u32> {
    std::fs::read_dir(run_dir.join(ACCEPTANCE_RECORDS_DIR))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            name.to_str()?.strip_prefix("round-")?.parse::<u32>().ok()
        })
        .max()
}

/// Persist a round record as exactly its own attempt, returning the path
/// written. Append-only: an existing attempt is never touched. The record
/// lands whole or not at all (staged, synced, renamed), and the directories
/// that hold it are synced, so a system crash cannot leave a torn record or
/// lose a written one. Whether the attempt is free is decided under the
/// recording-order lock, held until the record has landed: of two writers
/// of one attempt, the second is refused. The stage records a round with
/// [`record_round`], which never refuses a taken number.
pub fn write_round_record(
    run_dir: &Path,
    record: &AcceptanceRoundRecordV1,
) -> WorkflowResult<PathBuf> {
    let dir = round_dir(run_dir, record.round);
    std::fs::create_dir_all(&dir).map_err(|source| WorkflowError::io(&dir, source))?;
    progress::under_order_lock(run_dir, || {
        if attempt_taken(&dir, record.attempt)? {
            return Err(WorkflowError::StateCorrupt(format!(
                "acceptance record {} already exists (or was quarantined); round records are append-only",
                dir.join(attempt_file_name(record.attempt)).display()
            )));
        }
        land_locked(run_dir, &dir, record)
    })
}

/// Issue 316: records `record` for the round's owner, as the next FREE
/// attempt of its round. Under the recording-order lock, held until the
/// record has landed: when another writer took its attempt meanwhile (the
/// number was chosen when the round started), the record takes the next
/// free one instead, so the owner is never refused for a number. Then
/// `decide` runs on the record with the number it lands as: it fences the
/// writer (an owner a resume replaced is refused, and nothing lands) and
/// settles what the record says from the history it now sees, the other
/// writer's record included. An `Err` from it lands nothing.
pub fn record_round<T, E: From<WorkflowError>>(
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
    decide: impl FnOnce(&mut AcceptanceRoundRecordV1) -> Result<T, E>,
) -> Result<(PathBuf, T), E> {
    let dir = round_dir(run_dir, record.round);
    std::fs::create_dir_all(&dir).map_err(|source| WorkflowError::io(&dir, source))?;
    progress::under_order_lock(run_dir, || {
        if attempt_taken(&dir, record.attempt)? {
            let wanted = record.attempt;
            record.attempt = next_attempt(run_dir, record.round);
            if attempt_taken(&dir, record.attempt)? {
                return Err(WorkflowError::StateCorrupt(format!(
                    "acceptance round {} has no free attempt past {}",
                    record.round, record.attempt
                ))
                .into());
            }
            tracing::warn!(
                round = record.round,
                wanted,
                attempt = record.attempt,
                "another writer took this acceptance attempt; the record takes the next free one"
            );
        }
        let decided = decide(record)?;
        Ok((land_locked(run_dir, &dir, record)?, decided))
    })
}

/// Whether `attempt` of the round directory `dir` is taken: recorded, or
/// quarantined. The caller holds the order lock.
fn attempt_taken(dir: &Path, attempt: u32) -> WorkflowResult<bool> {
    let path = dir.join(attempt_file_name(attempt));
    Ok(path
        .try_exists()
        .map_err(|source| WorkflowError::io(&path, source))?
        || progress::quarantined_attempt(dir, attempt)?)
}

/// Lands `record` as its attempt of `dir`. The caller holds the order lock
/// and has checked the attempt is free.
fn land_locked(
    run_dir: &Path,
    dir: &Path,
    record: &AcceptanceRoundRecordV1,
) -> WorkflowResult<PathBuf> {
    let path = dir.join(attempt_file_name(record.attempt));
    let bytes = serde_json::to_vec_pretty(record)?;
    // The order entry first, synced: a record never exists without its
    // place in the recording order unless the log itself failed.
    progress::note_recorded_locked(run_dir, record.round, record.attempt);
    // The staging name is never an `attempt-*.json`: a crash before the
    // rename leaves no record.
    let staging = dir.join(format!(
        ".{}.{}.tmp",
        attempt_file_name(record.attempt),
        uuid::Uuid::new_v4()
    ));
    if let Err(error) = crate::store::write_atomic(&staging, &path, &bytes) {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }
    sync_record_dirs(run_dir, dir)?;
    Ok(path)
}

/// Syncs `dir` and each directory above it up to `run_dir`: any of them
/// `create_dir_all` may just have made, so each new entry survives a crash.
fn sync_record_dirs(run_dir: &Path, dir: &Path) -> WorkflowResult<()> {
    (dir.ancestors())
        .take_while(|ancestor| ancestor.starts_with(run_dir))
        .try_for_each(crate::store::sync_dir)
}

/// The most recent record: highest round, highest attempt. `None` when the
/// run never reached the acceptance stage (a legacy script, or a run that
/// stopped before it).
pub fn latest_round_record(
    run_dir: &Path,
) -> WorkflowResult<Option<(AcceptanceRoundRecordV1, PathBuf)>> {
    let Some(round) = highest_round(run_dir) else {
        return Ok(None);
    };
    let dir = round_dir(run_dir, round);
    let Some(attempt) = highest_attempt(&dir) else {
        return Ok(None);
    };
    let path = dir.join(attempt_file_name(attempt));
    let bytes = std::fs::read(&path).map_err(|source| WorkflowError::io(&path, source))?;
    let record: AcceptanceRoundRecordV1 = serde_json::from_slice(&bytes)?;
    Ok(Some((record, path)))
}

/// Path of a record relative to the run directory, for summaries.
pub fn relative_record_path(run_dir: &Path, path: &Path) -> String {
    path.strip_prefix(run_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
#[path = "acceptance_record_round_tests.rs"]
mod record_round_tests;
#[cfg(test)]
#[path = "acceptance_stage_tests.rs"]
mod tests;
