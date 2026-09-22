//! What a branch's declared focused tests do on the BASE commit, established
//! by the host before the coder is dispatched (Obs-31).
//!
//! # The gap this closes
//!
//! Live on wf-caac2ac3 the implementation run's base branch carried two
//! failing tests in a module no task in the set owned. Nothing ran a task's
//! declared filter on the base commit, so every coder whose filter covered
//! them re-discovered them and spent its read budget deciding they were old;
//! the verifier, seeing red tests outside the task's own, accepted by
//! judgement ("pre-existing, out of scope"); and the breakage rode to the
//! end of the run with no owner.
//!
//! # What is recorded, and where
//!
//! For every branch in a wave, each declared focused test command is run in
//! the branch's own pristine worktree (base commit, before any partial work
//! is resumed into it), in the environment the coder's own build/test calls
//! get. The failing test names are read from the runner's output
//! ([`super::test_baseline_parse`]); a command that names none records its
//! exit and last lines. The record lands under the run's v2 store at
//! `baseline-tests/<stage>/<branch>.json`; each command's verdict is also
//! cached at `baseline-tests/cache/<base sha>-<command hash>.json`, so a
//! resumed pass or a sibling branch declaring the same command never reruns
//! it against the same commit.
//!
//! # Who answers for a failure
//!
//! Each failing test is resolved to its file and the file to its owner
//! ([`super::test_baseline_owner`]). A file another task in the universe
//! declares is that task's: the failure is routed to it as a finding under
//! `baseline-tests/findings/<task>.json` — the queue the mandatory review's
//! final reducer merges into the host review findings, which is what
//! `remediateFindings` acts on — and the current coder is told to ignore
//! the test by name. Anything else (this task's file, nobody's file, a test
//! no file can be found for, a command that failed without naming one) is
//! the CURRENT task's obligation: it is told so, its scope is widened to
//! the file, and its verifier is told the task is not accepted while the
//! test is red. A file the task is forbidden to change is the one exception
//! to "otherwise yours": the coder cannot edit it, so the test is listed to
//! ignore and the failure recorded as unowned.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::v2::WorkflowV2ResultStore;

pub(crate) const SCHEMA_VERSION: u32 = 1;

/// Directory under the run's v2 store holding every baseline artefact.
const ROOT_DIR: &str = "baseline-tests";
const CACHE_DIR: &str = "cache";
const FINDINGS_DIR: &str = "findings";

/// One declared command's verdict on the base commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommandBaseline {
    pub command: String,
    pub base_commit: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    /// Test ids the runner reported failed; empty for a passing command and
    /// for one whose output names no test.
    pub failing_tests: Vec<String>,
    /// The last lines of output when the command failed without naming a
    /// test, or could not run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tail: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Served from the (base commit, command) cache rather than run again.
    #[serde(default)]
    pub cached: bool,
    /// Repo-relative files the command's error diagnostics point at
    /// (`--> path:line:col`, `Diff in path:line:`), for a failed command
    /// that names no test — a lint, a format check, a build with
    /// `-D warnings`. Empty for a passing command and for one whose output
    /// carries no location.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostic_files: Vec<String>,
}

impl CommandBaseline {
    /// The command passed on the base commit.
    pub(crate) fn passed(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out && self.error.is_none()
    }

    /// The command failed or has no verdict, and no test name explains it.
    pub(crate) fn failed_unattributed(&self) -> bool {
        !self.passed() && self.failing_tests.is_empty()
    }
}

/// A failure the current task answers for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BaselineObligation {
    /// The test id; `None` for a command that failed without naming one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_id: Option<String>,
    /// The file the test was resolved to, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub command: String,
}

/// A failure routed to another task that declares its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RoutedFailure {
    pub test_id: String,
    pub file: String,
    pub owner_task: String,
    pub command: String,
}

/// A failure the current task is told to ignore without an owner: its file
/// is forbidden to this task and no other task declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IgnoredFailure {
    pub test_id: String,
    pub file: String,
    pub reason: String,
}

/// A declared non-test command that is red on the base commit because of
/// diagnostics in files outside the task's target set (Issue-64). Not the
/// coder's to fix: it is told so, and its verifier accepts a `pre_existing`
/// claim on the command whose diagnostics stay within these files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreExistingDiagnostics {
    pub command: String,
    /// Repo-relative, sorted.
    pub files: Vec<String>,
    /// The task that declares each file, where one does (`file` → task).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<(String, String)>,
}

/// The whole baseline of one branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BranchBaseline {
    pub schema_version: u32,
    pub stage_id: String,
    pub branch_id: String,
    pub base_commit: String,
    pub canonical_task_ids: Vec<String>,
    pub commands: Vec<CommandBaseline>,
    #[serde(default)]
    pub obligations: Vec<BaselineObligation>,
    #[serde(default)]
    pub routed: Vec<RoutedFailure>,
    #[serde(default)]
    pub ignored: Vec<IgnoredFailure>,
    /// Failures other branches' baselines routed to this branch's tasks,
    /// found in their filters, in files this task declares.
    #[serde(default)]
    pub inherited: Vec<BaselineObligation>,
    /// Declared commands red on the base commit for out-of-scope
    /// diagnostics only (Issue-64); one entry per such command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_existing: Vec<PreExistingDiagnostics>,
}

impl BranchBaseline {
    /// Nothing was declared, so nothing was baselined.
    pub(crate) fn is_empty(&self) -> bool {
        self.commands.is_empty() && self.inherited.is_empty()
    }

    /// Every file this task must be allowed to change to meet its
    /// obligations, sorted and deduplicated.
    pub(crate) fn obligation_files(&self) -> Vec<String> {
        let mut files: Vec<String> = self
            .obligations
            .iter()
            .chain(&self.inherited)
            .filter_map(|o| o.file.clone())
            .collect();
        files.sort();
        files.dedup();
        files
    }

    /// The test ids the verifier must see pass, sorted.
    pub(crate) fn must_pass(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .obligations
            .iter()
            .chain(&self.inherited)
            .filter_map(|o| o.test_id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

fn root(store: &WorkflowV2ResultStore) -> PathBuf {
    store.root().join(ROOT_DIR)
}

fn segment(raw: &str) -> String {
    super::sanitize_v2_path_segment(raw)
}

pub(crate) fn record_path(
    store: &WorkflowV2ResultStore,
    stage_id: &str,
    branch_id: &str,
) -> PathBuf {
    root(store)
        .join(segment(stage_id))
        .join(format!("{}.json", segment(branch_id)))
}

fn cache_path(store: &WorkflowV2ResultStore, base_commit: &str, command: &str) -> PathBuf {
    let sha: String = base_commit.chars().take(12).collect();
    let hash = blake3::hash(command.as_bytes()).to_hex();
    root(store)
        .join(CACHE_DIR)
        .join(format!("{}-{}.json", segment(&sha), &hash[..16]))
}

fn findings_path(store: &WorkflowV2ResultStore, task_id: &str) -> PathBuf {
    root(store)
        .join(FINDINGS_DIR)
        .join(format!("{}.json", segment(task_id)))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn save_record(store: &WorkflowV2ResultStore, record: &BranchBaseline) {
    let path = record_path(store, &record.stage_id, &record.branch_id);
    if let Err(error) = write_json(&path, record) {
        eprintln!("baseline tests: cannot write {}: {error}", path.display());
    }
}

#[cfg(test)]
pub(crate) fn load_record(
    store: &WorkflowV2ResultStore,
    stage_id: &str,
    branch_id: &str,
) -> Option<BranchBaseline> {
    read_json(&record_path(store, stage_id, branch_id))
}

/// The cached verdict for `command` on `base_commit`, marked as such.
pub(crate) fn cached_command(
    store: &WorkflowV2ResultStore,
    base_commit: &str,
    command: &str,
) -> Option<CommandBaseline> {
    let mut hit: CommandBaseline = read_json(&cache_path(store, base_commit, command))?;
    if hit.base_commit != base_commit || hit.command != command {
        return None;
    }
    hit.cached = true;
    Some(hit)
}

pub(crate) fn cache_command(store: &WorkflowV2ResultStore, verdict: &CommandBaseline) {
    let path = cache_path(store, &verdict.base_commit, &verdict.command);
    if let Err(error) = write_json(&path, verdict) {
        eprintln!("baseline tests: cannot cache {}: {error}", path.display());
    }
}

/// Every branch record in the store, newest first by modification time.
pub(crate) fn all_records(store: &WorkflowV2ResultStore) -> Vec<BranchBaseline> {
    let mut found: Vec<(std::time::SystemTime, BranchBaseline)> = Vec::new();
    let Ok(stages) = std::fs::read_dir(root(store)) else {
        return Vec::new();
    };
    for stage in stages.flatten() {
        let name = stage.file_name();
        if name == CACHE_DIR || name == FINDINGS_DIR || !stage.path().is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(stage.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if let Some(record) = read_json::<BranchBaseline>(&path) {
                found.push((modified, record));
            }
        }
    }
    found.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    found.into_iter().map(|(_, record)| record).collect()
}

/// Append `finding` to the owner task's queue unless an identical
/// (`test_id`, owner) entry is already there.
pub(crate) fn route_finding(store: &WorkflowV2ResultStore, owner_task: &str, finding: Value) {
    let path = findings_path(store, owner_task);
    let mut queue: Vec<Value> = read_json(&path).unwrap_or_default();
    let same = |a: &Value, b: &Value| a.get("test_id") == b.get("test_id");
    if queue.iter().any(|known| same(known, &finding)) {
        return;
    }
    queue.push(finding);
    if let Err(error) = write_json(&path, &queue) {
        eprintln!(
            "baseline tests: cannot route finding to {}: {error}",
            path.display()
        );
    }
}

/// The findings other branches routed to `task_id`.
pub(crate) fn routed_findings_for_task(store: &WorkflowV2ResultStore, task_id: &str) -> Vec<Value> {
    read_json(&findings_path(store, task_id)).unwrap_or_default()
}

/// Every routed finding in the run, in task order.
pub(crate) fn all_routed_findings(store: &WorkflowV2ResultStore) -> Vec<Value> {
    let Ok(entries) = std::fs::read_dir(root(store).join(FINDINGS_DIR)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| read_json::<Vec<Value>>(path))
        .flatten()
        .collect()
}

#[cfg(test)]
#[path = "test_baseline_lint_tests.rs"]
mod lint_tests;
#[cfg(test)]
#[path = "test_baseline_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "test_baseline_verification_tests.rs"]
mod verification_tests;
