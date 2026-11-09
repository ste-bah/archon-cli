//! A verifier's own failing test command, run by the HOST (Issue-118): once
//! at the RUN's base commit and once on the tree the verifier judged.
//!
//! The base-commit rule (`verification::baseline_rule`) refuses an accepted
//! verdict that leaves any test red that no other task owns. A verifier may
//! run more than its task's declared filter -- a crate's whole library suite
//! -- and every test that command names failing is held against the branch,
//! including tests already failing when the run began, in files the branch
//! may not write. In a recorded run, a residual round whose write scope was
//! one file was refused over seven such tests.
//!
//! What a test did is never taken from the agent's report here. The host runs
//! the command itself:
//!
//! - at the run's base commit -- the checkout's `HEAD` the run's first
//!   `repository_bound` event recorded -- in an isolated committed-tree copy;
//! - on the JUDGED tree -- an isolated copy of the recorded commit, and
//!   only while its `HEAD` is still the commit the verifier judged.
//!
//! Commands run verbatim only when authorized by the frozen declarations.
//! Completed output supplies observed test ids and normalized failure
//! signatures. A readable summary cannot prove that unobserved tests ran;
//! those remain unknown. Only isolated judged runs are reused from cache.
//! Unreadable output, launch errors, and timeouts yield no per-test verdict.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::test_baseline_parse::failing_tests;
use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::v2::WorkflowV2ResultStore;

/// Directory under `baseline-tests/` holding these verdicts.
const RUN_BASE_DIR: &str = "run-base";
/// Most distinct commands the host runs for ONE branch; the rest are named
/// on the branch as not run, and excuse nothing.
pub(crate) const MAX_COMMANDS_PER_BRANCH: usize = 4;

/// Which tree a verdict describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tree {
    /// The run's base commit, in an isolated committed-tree copy.
    RunBase,
    /// An isolated copy at the commit the verifier judged.
    Judged,
}

impl Tree {
    fn tag(self) -> &'static str {
        match self {
            Self::RunBase => "base",
            Self::Judged => "judged",
        }
    }
}

/// One command's host verdict on one tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HostRunVerdict {
    pub command: String,
    pub commit: String,
    pub exit_code: Option<i32>,
    /// Test ids the runner reported failed, sorted.
    #[serde(default)]
    pub failing_tests: Vec<String>,
    /// Per failing test: its normalised failure signature.
    #[serde(default)]
    pub signatures: BTreeMap<String, String>,
    /// Per failing test: the repo-relative files its failure locations name.
    #[serde(default)]
    pub failure_files: BTreeMap<String, Vec<String>>,
    /// The failures the harness itself counted, summed over its summary
    /// lines; `None` when it printed no count the host can read.
    #[serde(default)]
    pub failed_count: Option<usize>,
    /// Test ids the runner named passed, and named ignored (Issue-114: a test
    /// that passed at the base and is ignored at the tip was hidden, not
    /// fixed). Empty in a verdict cached before they were kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passed_tests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored_tests: Vec<String>,
    /// Written by a host that keeps those ids (`false` in an older cache
    /// entry, whose empty lists say nothing).
    #[serde(default)]
    pub ids_kept: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub verbatim: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub per_test_verdict: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub isolated: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// The run's base commit: the `HEAD` its first `repository_bound` event
/// recorded, read from the run directory the v2 store sits in.
pub(crate) fn run_base_commit(store: &WorkflowV2ResultStore) -> Option<String> {
    let events = store.root().parent()?.join("events.jsonl");
    let bytes = std::fs::read(&events).ok()?;
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let event = match serde_json::from_slice::<serde_json::Value>(line) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(path = %events.display(), line = index + 1, %error,
                    "Skipping malformed workflow event during repository binding lookup");
                continue;
            }
        };
        if event["detail"]["event"] == "repository_bound" {
            return event["detail"]["head"]
                .as_str()
                .map(str::to_string)
                .filter(|head| !head.trim().is_empty());
        }
    }
    None
}

fn verdict_path(store: &WorkflowV2ResultStore, tree: Tree, commit: &str, command: &str) -> PathBuf {
    let sha: String = commit.chars().take(12).collect();
    let hash =
        blake3::hash(format!("{}\0{command}", super::test_baseline::EXECUTION_LOGIC).as_bytes())
            .to_hex();
    store
        .root()
        .join("baseline-tests")
        .join(RUN_BASE_DIR)
        .join(format!(
            "{}-{}-{}.json",
            tree.tag(),
            super::sanitize_v2_path_segment(&sha),
            &hash[..16]
        ))
}

/// The host's own verdict for `command` on `tree` at `commit`, if it ran one.
pub(crate) fn cached(
    store: &WorkflowV2ResultStore,
    tree: Tree,
    commit: &str,
    command: &str,
) -> Option<HostRunVerdict> {
    let path = verdict_path(store, tree, commit, command);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cached verdict could not be read");
            return None;
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cached verdict could not be decoded");
            return None;
        }
    };
    if value
        .get("identity_digits_preserved")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        tracing::info!(%command, "cached verdict does not attest digit-preserving identity evidence");
        return None;
    }
    let mut verdict: HostRunVerdict = match serde_json::from_value(value) {
        Ok(verdict) => verdict,
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "cached verdict could not be decoded");
            return None;
        }
    };
    if tree == Tree::Judged && !verdict.isolated {
        tracing::debug!(%command, "stored verdict does not record isolated execution");
        return None;
    }
    if !verdict.verbatim {
        tracing::debug!(%command, "stored verdict does not record verbatim execution");
        return None;
    }
    if let Some(stored) = verdict.signatures.remove("")
        && let Some(identity) = super::test_baseline_parse::failure_identity(&stored, Path::new(""))
    {
        verdict.signatures.insert(String::new(), identity);
    }
    (verdict.commit == commit && verdict.command == command).then_some(verdict)
}

pub(crate) fn cached_per_test(
    store: &WorkflowV2ResultStore,
    tree: Tree,
    commit: &str,
    command: &str,
    frozen: &FrozenCommands,
) -> Option<HostRunVerdict> {
    let command = frozen.command(command).ok()?;
    cached(store, tree, commit, command).filter(|verdict| verdict.per_test_verdict)
}

pub(crate) fn cache(store: &WorkflowV2ResultStore, tree: Tree, verdict: &HostRunVerdict) {
    let path = verdict_path(store, tree, &verdict.commit, &verdict.command);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let tmp = path.with_extension("json.tmp");
        let mut value = serde_json::to_value(verdict)?;
        value["identity_digits_preserved"] = serde_json::Value::Bool(true);
        std::fs::write(&tmp, serde_json::to_vec_pretty(&value)?)?;
        std::fs::rename(&tmp, &path)
    };
    if let Err(error) = write() {
        eprintln!("baseline tests: cannot write {}: {error}", path.display());
    }
}

/// Each host-runnable command of `commands` on `tree` at `commit`, from the
/// cache or run once. Both trees run in isolated committed-tree copies.
/// The judged revision is checked against the live head before copying. A command without a verdict is
/// absent from the map.
pub(crate) async fn host_verdicts(
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
    repository_root: &Path,
    tree: Tree,
    commit: &str,
    commands: &[String],
    frozen: &FrozenCommands,
) -> BTreeMap<String, HostRunVerdict> {
    let mut verdicts = BTreeMap::new();
    let mut pending = Vec::new();
    for reported in commands {
        let command = match frozen.command(reported) {
            Ok(command) => command,
            Err(reason) => {
                tracing::debug!(%reported, %reason, "command was not run");
                continue;
            }
        };
        match cached(store, tree, commit, command).filter(|run| run.verbatim) {
            Some(hit) => {
                verdicts.insert(reported.clone(), hit);
            }
            None => pending.push((reported.clone(), command.to_string())),
        }
    }
    if pending.is_empty() {
        return verdicts;
    }
    let sha: String = commit.chars().take(12).collect();
    let scratch = store.root().join("worktrees").join(format!(
        "run-base-{}",
        super::sanitize_v2_path_segment(&sha)
    ));
    if tree == Tree::Judged
        && crate::repository_record::git_head(repository_root)
            .ok()
            .as_deref()
            != Some(commit)
    {
        return verdicts;
    }
    if !add_worktree(repository_root, &scratch, commit) {
        return verdicts;
    }
    let workdir = scratch.clone();
    for (reported, command) in pending {
        let run = super::test_baseline_run::run_in_worktree(
            dispatch,
            &workdir,
            &command,
            Some(store.run_root()),
        )
        .await;
        if run.error.is_some()
            || run.timed_out
            || run.exit_code.is_none()
            || crate::acceptance_check_shell_unset::never_pass(
                &command,
                run.exit_code.unwrap_or(-1),
                &run.output,
            )
        {
            tracing::warn!(%command, error = ?run.error, timed_out = run.timed_out, exit_code = ?run.exit_code, "the host run did not produce a usable completion verdict");
            continue;
        }
        let failing = failing_tests(&run.output);
        let mut signatures: BTreeMap<String, String> = failing
            .iter()
            .map(|id| (id.clone(), panics::signature_at(&run.output, id, &workdir)))
            .collect();
        if let Some(identity) = super::test_baseline_parse::failure_identity(&run.output, &workdir)
        {
            signatures.insert(String::new(), identity);
        }
        let mut failure_files: BTreeMap<String, Vec<String>> = failing
            .iter()
            .map(|id| (id.clone(), panic_files(&run.output, id, &workdir)))
            .collect();
        failure_files.insert(
            String::new(),
            super::test_baseline_parse::diagnostic_files_for(&run.output, &workdir, &command),
        );
        let verdict = HostRunVerdict {
            command: command.clone(),
            commit: commit.to_string(),
            exit_code: run.exit_code,
            signatures,
            failure_files,
            failing_tests: failing,
            failed_count: failed_count(&run.output),
            passed_tests: super::test_baseline_parse::passed_tests(&run.output),
            ignored_tests: super::test_baseline_parse::ignored_tests(&run.output),
            ids_kept: true,
            verbatim: true,
            per_test_verdict: harness_reported(&run.output),
            isolated: true,
        };
        cache(store, tree, &verdict);
        verdicts.insert(reported, verdict);
    }
    remove_worktree(repository_root, &scratch);
    verdicts
}

#[path = "test_baseline_frozen.rs"]
mod frozen;
use super::test_baseline_parse::{failed_count, harness_reported};
pub(crate) use frozen::{CommandNotRun, FrozenCommands, host_runnable};

#[path = "test_baseline_run_base_git.rs"]
mod git;
pub(in crate::v2) use git::{add_worktree, remove_worktree};

#[path = "test_baseline_run_base_panics.rs"]
mod panics;
#[cfg(test)]
pub(crate) use panics::signature;
pub(crate) use panics::{panic_files, signature_at};

#[cfg(test)]
#[path = "test_baseline_run_base_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "test_baseline_host_rule_tests.rs"]
mod rule_tests;

#[cfg(all(test, unix))]
#[path = "test_baseline_isolated_tests.rs"]
mod isolated_tests;
