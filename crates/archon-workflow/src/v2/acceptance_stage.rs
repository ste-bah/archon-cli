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
/// Hard ceiling on acceptance remediation rounds, matching the review
/// remediation contract's own bound.
pub const ACCEPTANCE_MAX_ROUNDS: u32 = 3;

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
}

impl AcceptanceCheckRecordV1 {
    pub fn failing(&self) -> bool {
        self.status != AcceptanceCheckStatus::Passed
    }
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
    /// Whether the script's loop ends here: no failing checks, the last
    /// permitted round, or nothing left that a task could remediate.
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

    /// The round blocks completion: a failing check or a stage that could not
    /// evaluate. A round with no contract to run passes vacuously and says so
    /// through `contract_present`.
    pub fn blocks_completion(&self) -> bool {
        !self.operational_errors.is_empty() || self.checks.iter().any(|check| check.failing())
    }

    /// Whether any failing check is owned by a task, so remediation has
    /// somewhere to go.
    pub fn has_remediable_failures(&self) -> bool {
        self.failing_checks()
            .iter()
            .any(|check| !check.owning_tasks.is_empty())
    }
}

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
/// already on disk. Records are never overwritten.
pub fn next_attempt(run_dir: &Path, round: u32) -> u32 {
    highest_attempt(&round_dir(run_dir, round)).map_or(1, |attempt| attempt + 1)
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

/// Persist a round record as the next attempt of its round, returning the
/// path written. Append-only: an existing attempt is never touched.
pub fn write_round_record(
    run_dir: &Path,
    record: &AcceptanceRoundRecordV1,
) -> WorkflowResult<PathBuf> {
    let dir = round_dir(run_dir, record.round);
    std::fs::create_dir_all(&dir).map_err(|source| WorkflowError::io(&dir, source))?;
    let path = dir.join(attempt_file_name(record.attempt));
    if path.exists() {
        return Err(WorkflowError::StateCorrupt(format!(
            "acceptance record {} already exists; round records are append-only",
            path.display()
        )));
    }
    let bytes = serde_json::to_vec_pretty(record)?;
    std::fs::write(&path, bytes).map_err(|source| WorkflowError::io(&path, source))?;
    Ok(path)
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
        .display()
        .to_string()
}

#[cfg(test)]
#[path = "acceptance_stage_tests.rs"]
mod tests;
