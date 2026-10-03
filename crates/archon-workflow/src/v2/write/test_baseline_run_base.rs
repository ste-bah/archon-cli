//! A verifier's own failing test command, run by the HOST (Issue-118): once
//! at the RUN's base commit and once on the tree the verifier judged.
//!
//! The base-commit rule (`verification::baseline_rule`) refuses an accepted
//! verdict that leaves any test red that no other task owns. A verifier may
//! run more than its task's declared filter -- a crate's whole library suite
//! -- and every test that command names failing is held against the branch,
//! including tests already failing when the run began, in files the branch
//! may not write. Live on wf-0ddadd81 a residual round whose write scope was
//! one file was refused over seven such tests.
//!
//! What a test did is never taken from the agent's report here. The host runs
//! the command itself:
//!
//! - at the run's base commit -- the checkout's `HEAD` the run's first
//!   `repository_bound` event recorded -- in a throwaway detached worktree;
//! - on the JUDGED tree -- the checkout the verifier ran in, in place, and
//!   only while its `HEAD` is still the commit the verifier judged.
//!
//! Each run yields the failing tests its runner named and, per test, its
//! failure SIGNATURE: the panic location and first message line, with
//! absolute paths and temporary names normalised. Only a plain test-runner
//! invocation is run (`cargo test ...` / `cargo nextest run ...`, plain
//! words, known options only: `args`), always with `--no-fail-fast`. A run
//! whose harness did not report a verdict for every test binary (a build
//! failure, a timeout, a kill) is no verdict: it is never cached and excuses
//! nothing. Verdicts are cached per (tree kind, commit,
//! command) under `baseline-tests/run-base/`.

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
    /// The run's base commit, in a throwaway detached worktree.
    RunBase,
    /// The verifier's own checkout, in place, at the commit it judged.
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
    let hash = blake3::hash(command.as_bytes()).to_hex();
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
    let bytes = std::fs::read(verdict_path(store, tree, commit, command)).ok()?;
    serde_json::from_slice::<HostRunVerdict>(&bytes)
        .ok()
        .filter(|verdict| verdict.commit == commit && verdict.command == command)
}

pub(crate) fn cache(store: &WorkflowV2ResultStore, tree: Tree, verdict: &HostRunVerdict) {
    let path = verdict_path(store, tree, &verdict.commit, &verdict.command);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(verdict)?)?;
        std::fs::rename(&tmp, &path)
    };
    if let Err(error) = write() {
        eprintln!("baseline tests: cannot write {}: {error}", path.display());
    }
}

/// Each host-runnable command of `commands` on `tree` at `commit`, from the
/// cache or run once. `Tree::Judged` runs in `repository_root` itself and
/// only while its `HEAD` is `commit`; `Tree::RunBase` runs in a throwaway
/// detached worktree removed afterwards. A command without a verdict is
/// absent from the map.
pub(crate) async fn host_verdicts(
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
    repository_root: &Path,
    tree: Tree,
    commit: &str,
    commands: &[String],
) -> BTreeMap<String, HostRunVerdict> {
    let mut verdicts = BTreeMap::new();
    let mut pending = Vec::new();
    for command in commands.iter().filter(|command| host_runnable(command)) {
        match cached(store, tree, commit, command) {
            Some(hit) => {
                verdicts.insert(command.clone(), hit);
            }
            None => pending.push(command.clone()),
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
    let workdir = match tree {
        Tree::Judged => {
            let head = crate::repository_record::git_head(repository_root).ok();
            if head.as_deref() != Some(commit) {
                return verdicts;
            }
            repository_root.to_path_buf()
        }
        Tree::RunBase => {
            if !add_worktree(repository_root, &scratch, commit) {
                return verdicts;
            }
            scratch.clone()
        }
    };
    for command in pending {
        let run = super::test_baseline_run::run_in_worktree(
            dispatch,
            &workdir,
            &complete_run(&command),
            Some(store.run_root()),
        )
        .await;
        if run.error.is_some() || run.timed_out || !harness_reported(&run.output) {
            continue;
        }
        let failing = failing_tests(&run.output);
        let verdict = HostRunVerdict {
            command: command.clone(),
            commit: commit.to_string(),
            exit_code: run.exit_code,
            signatures: failing
                .iter()
                .map(|id| (id.clone(), signature(&run.output, id)))
                .collect(),
            failure_files: failing
                .iter()
                .map(|id| (id.clone(), panic_files(&run.output, id, &workdir)))
                .collect(),
            failing_tests: failing,
            failed_count: failed_count(&run.output),
            passed_tests: super::test_baseline_parse::passed_tests(&run.output),
            ignored_tests: super::test_baseline_parse::ignored_tests(&run.output),
            ids_kept: true,
        };
        cache(store, tree, &verdict);
        verdicts.insert(command, verdict);
    }
    if tree == Tree::RunBase {
        remove_worktree(repository_root, &scratch);
    }
    verdicts
}

#[path = "test_baseline_run_base_args.rs"]
mod args;
pub(crate) use args::host_runnable;
use args::{complete_run, failed_count, harness_reported};

#[path = "test_baseline_run_base_git.rs"]
mod git;
use git::{add_worktree, remove_worktree};

#[path = "test_baseline_run_base_panics.rs"]
mod panics;
pub(crate) use panics::{panic_files, signature};

#[cfg(test)]
#[path = "test_baseline_run_base_tests.rs"]
pub(crate) mod tests;
