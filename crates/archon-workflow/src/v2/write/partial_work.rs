//! Work a write branch produced but could not finish.
//!
//! A branch that runs out of its call budget returns no manifest, so the wave
//! treats it as having contributed nothing and removes its worktree. Six hours
//! of code disappeared that way on a live run, and the next attempt at the
//! same task started from zero. Here the diff is captured beside the stage's
//! patches, recorded on the branch outcome, and applied onto the next fresh
//! worktree prepared for the same canonical task, which is told so.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::v2::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result, WorkflowV2Status};
use crate::write_coordinator::worktree_isolation::run_git;
use crate::{WorkflowError, WorkflowResult, WorkflowV2ResultStore};

pub(crate) const DATA_KEY: &str = "partial_work";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialWork {
    pub patch_path: PathBuf,
    pub files: Vec<String>,
    pub bytes: u64,
    pub baseline_commit: String,
}

fn git(args: &[&str], cwd: &Path) -> WorkflowResult<Vec<u8>> {
    run_git(args, cwd)
        .map(|output| output.stdout)
        .map_err(|error| WorkflowError::StageFailed(format!("partial work git: {error}")))
}

/// Capture everything the branch changed against its sealed baseline: tracked
/// edits and new files alike, ignored paths (build output) excluded by git.
/// The `<item>.partial.json` sidecar written beside the patch names the
/// `task_ids` it is for, so the patch stays resolvable when every outcome
/// record that mentioned it has been superseded (Issue-18).
pub(crate) fn capture_partial_work(
    workspace_root: &Path,
    run_root: &Path,
    stage_id: &str,
    item_id: &str,
    task_ids: &[String],
) -> WorkflowResult<Option<PartialWork>> {
    let baseline_commit = String::from_utf8_lossy(&git(&["rev-parse", "HEAD"], workspace_root)?)
        .trim()
        .to_string();
    git(&["add", "-N", "-A"], workspace_root)?;
    let patch = git(&["diff", "--binary", "--no-color", "HEAD"], workspace_root)?;
    if patch.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let names = git(&["diff", "--name-only", "HEAD"], workspace_root)?;
    let files: Vec<String> = String::from_utf8_lossy(&names)
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    let dir = run_root
        .join("write-coordination")
        .join("stages")
        .join(stage_id)
        .join("partial");
    std::fs::create_dir_all(&dir).map_err(|error| WorkflowError::io(&dir, error))?;
    let patch_path = dir.join(format!("{item_id}.patch"));
    std::fs::write(&patch_path, &patch).map_err(|error| WorkflowError::io(&patch_path, error))?;
    let partial = PartialWork {
        patch_path,
        files,
        bytes: patch.len() as u64,
        baseline_commit,
    };
    super::partial_work_lookup::write_sidecar(stage_id, item_id, task_ids, &partial)?;
    Ok(Some(partial))
}

/// A branch keeps its partial work when it ends without a manifest and without
/// acceptance: a budget timeout, an unhandled error, a rejected envelope.
pub(crate) fn branch_keeps_partial_work(result: &WorkflowV2Result, has_manifest: bool) -> bool {
    !has_manifest
        && !matches!(
            result.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        )
}

pub(crate) fn record_partial_work(result: &mut WorkflowV2Result, partial: &PartialWork) {
    if !result.data.is_object() {
        result.data = serde_json::json!({});
    }
    result.data[DATA_KEY] = serde_json::to_value(partial).unwrap_or_default();
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!(
            "partial work retained: {} file(s), {} bytes, applied to the next attempt at this task",
            partial.files.len(),
            partial.bytes
        ),
    ));
}

/// The most recent partial patch any earlier branch in this run left for one
/// of the given canonical task ids: sidecars in the partial directory, then
/// current and superseded outcome records (`partial_work_lookup`).
pub(crate) fn latest_partial_for_tasks(
    v2_store: &WorkflowV2ResultStore,
    task_ids: &[String],
) -> Option<PartialWork> {
    super::partial_work_lookup::latest_partial_for_tasks(v2_store, task_ids)
}

/// Apply onto a fresh worktree. Three-way so a baseline that moved on (a wave
/// commit landed in between) still takes the parts that apply; a failure is
/// returned, never fatal: the caller records it and the agent starts clean.
pub(crate) fn apply_partial_work(
    workspace_root: &Path,
    partial: &PartialWork,
) -> WorkflowResult<()> {
    let path = partial.patch_path.to_string_lossy().into_owned();
    git(&["apply", "--3way", "--allow-empty", &path], workspace_root).map(|_| ())
}

/// Find the newest partial work for the tasks this branch owns and lay it into
/// the fresh workspace. A patch that no longer applies is dropped, not fatal:
/// the agent starts clean and the outcome that carried it still says so.
pub(crate) fn resume_into_workspace(
    v2_store: &WorkflowV2ResultStore,
    universe: Option<&crate::task_universe::WorkflowV2TaskUniverse>,
    branch_input: &serde_json::Value,
    workspace_root: &Path,
) -> Option<PartialWork> {
    let source = branch_input.get("item").unwrap_or(branch_input);
    let task_ids =
        crate::generated_contract::canonical_task_ids_from_generated_value(source, universe);
    if task_ids.is_empty() {
        return None;
    }
    // A task that has since landed needs no resume: its accepted work is in
    // the baseline, and an older partial laid over it would only conflict.
    let outcomes = v2_store.load_branch_outcomes().unwrap_or_default();
    let landed = super::dependency_gate::landed_task_ids(&outcomes);
    if task_ids.iter().any(|id| landed.contains(id)) {
        return None;
    }
    let partial = latest_partial_for_tasks(v2_store, &task_ids)?;
    match apply_partial_work(workspace_root, &partial) {
        Ok(()) => Some(partial),
        Err(_) => {
            // A three-way apply that fails can leave conflict markers behind.
            // The workspace is ours and sealed at HEAD: put it back exactly.
            let _ = git(&["reset", "--hard", "HEAD", "--quiet"], workspace_root);
            let _ = git(&["clean", "-fdq"], workspace_root);
            None
        }
    }
}

/// What the host tells a write agent before the task text: how long it has,
/// that unfinished work survives the cut, and, when there is one, the partial
/// it is continuing from. An agent told none of this reads for an hour and is
/// cut with nothing to keep.
pub(crate) fn with_host_preamble(
    task: &str,
    budget: Option<std::time::Duration>,
    resumed: Option<&PartialWork>,
    memory: &super::session_memory::SessionMemory,
) -> String {
    host_preamble(task, budget, resumed, memory, false)
}

/// The same preamble for a session restarted MID-attempt: the transport
/// dropped or the host cut the previous session, and the branch re-asks in the
/// same worktree. The partial is the agent's own work from minutes ago, so it
/// is told so rather than being told an earlier attempt left it.
pub(crate) fn with_restart_preamble(
    task: &str,
    budget: Option<std::time::Duration>,
    partial: Option<&PartialWork>,
    memory: &super::session_memory::SessionMemory,
) -> String {
    host_preamble(task, budget, partial, memory, true)
}

/// `memory` is what the previous session tried — its refused calls and its
/// last few tool calls — rendered after the partial-work sentence. Without it
/// a resumed session re-tried, within minutes, the very calls the host had
/// refused the session before (Obs-8).
fn host_preamble(
    task: &str,
    budget: Option<std::time::Duration>,
    resumed: Option<&PartialWork>,
    memory: &super::session_memory::SessionMemory,
    same_attempt: bool,
) -> String {
    let mut parts = Vec::new();
    if let Some(budget) = budget {
        parts.push(format!(
            "Time budget: this call has {} minutes of wall clock in total, including every tool call and test run. Write the deliverable files first and verify them after; do not spend the budget reading. Work left in the workspace when the budget ends is kept and handed to the next attempt at this task, so partial files are worth more than a complete investigation.",
            budget.as_secs().div_ceil(60)
        ));
    }
    if let Some(partial) = resumed {
        let origin = if same_attempt {
            "This is the same attempt, restarted after the model connection ended; the workspace is exactly as you left it."
        } else {
            "A previous attempt at this task ran out of time before finishing."
        };
        parts.push(format!(
            "{origin} Its uncommitted work ({} file(s)) has been applied to this workspace: {}. Continue from that work; do not start over, and do not discard it unless it is wrong.",
            partial.files.len(),
            partial.files.join(", ")
        ));
    }
    if let Some(section) = memory.render() {
        parts.push(section);
    }
    if parts.is_empty() {
        return task.to_string();
    }
    format!("{}\n\n{task}", parts.join("\n\n"))
}

#[cfg(test)]
pub(crate) fn with_resume_preamble(task: &str, resumed: Option<&PartialWork>) -> String {
    with_host_preamble(task, None, resumed, &Default::default())
}

/// The wall clock the agent will actually run into on its next dispatch.
///
/// Two limits end a write call and neither knows about the other: the host
/// cancels one dispatch at its per-dispatch timeout, and the branch loop stops
/// re-dispatching once the total call budget is spent. The prompt used to
/// render only the second, so it promised 240 minutes to a session the host
/// ended at 120. The truthful number is the smaller of the per-dispatch limit
/// and whatever the total has left after `elapsed`; `None` only when neither
/// limit exists.
pub(crate) fn effective_call_budget(
    dispatch_timeout: Option<std::time::Duration>,
    call_time_budget: Option<std::time::Duration>,
    elapsed: std::time::Duration,
) -> Option<std::time::Duration> {
    let remaining = call_time_budget.map(|budget| budget.saturating_sub(elapsed));
    match (dispatch_timeout, remaining) {
        (Some(per_dispatch), Some(remaining)) => Some(per_dispatch.min(remaining)),
        (per_dispatch, remaining) => per_dispatch.or(remaining),
    }
}

/// What a write branch needs to re-render its task for a session started
/// mid-attempt.
///
/// The task text is rendered once, when the branch starts against a clean
/// worktree, and the re-ask loop used to send that same text into every fresh
/// session after a transport drop or host timeout. The restarted agent was
/// never told the eight files it had already written were sitting in its
/// workspace; it found them only because it happened to run `git status`.
pub(crate) struct BranchTaskRefresh {
    /// The task as rendered before the read-set and host preambles.
    pub(crate) base_task: String,
    pub(crate) task_ids: Vec<String>,
    pub(crate) run_root: PathBuf,
    pub(crate) stage_id: String,
    pub(crate) item_id: String,
}

impl BranchTaskRefresh {
    /// The task for a fresh session in the same worktree: the current partial
    /// work (captured the same way a finished branch's is), the recorded read
    /// set, what the ended session had refused and last ran (from the
    /// sidecar the guard keeps under `call_id`, `last_calls` of them), and
    /// the budget this session actually has.
    pub(crate) fn restarted_task(
        &self,
        v2_store: &WorkflowV2ResultStore,
        workspace_root: &Path,
        call_id: &str,
        last_calls: usize,
        budget: Option<std::time::Duration>,
    ) -> String {
        let partial = capture_partial_work(
            workspace_root,
            &self.run_root,
            &self.stage_id,
            &self.item_id,
            &self.task_ids,
        )
        .ok()
        .flatten();
        let with_reads = crate::v2::write_read_set::with_retry_preamble(
            &self.base_task,
            v2_store,
            &self.task_ids,
        );
        let memory =
            super::session_memory::SessionMemory::for_branch(v2_store, call_id, last_calls);
        with_restart_preamble(&with_reads, budget, partial.as_ref(), &memory)
    }
}

#[cfg(test)]
#[path = "partial_work_tests.rs"]
mod tests;
