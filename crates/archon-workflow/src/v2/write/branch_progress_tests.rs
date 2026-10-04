//! Issue 263: a write branch is bounded by no progress, never by a total
//! count of re-asks or a total wall clock. A branch that stops progressing
//! reports its outcome (round 3, decision B): the wave captures its work and
//! remediation proceeds; the run is not paused at the branch.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use crate::{RunStatus, WorkflowStore, WorkflowV2HostMethod};

/// Rejects the patch for size `rejections` times, each overshoot smaller
/// than the last unless `stuck`, then accepts it.
struct Shrinking {
    attempts: AtomicUsize,
    rejections: usize,
    stuck: bool,
    budget: Option<Duration>,
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for Shrinking {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    fn call_time_budget(&self) -> Option<Duration> {
        self.budget
    }
    async fn run_call(
        &self,
        _task: &str,
        _: Option<String>,
        _: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n >= self.rejections {
            return Ok(WorkflowV2Result::accepted("fits the cap"));
        }
        let lines = if self.stuck { 600 } else { 600 - n };
        Err(WorkflowError::port(format!(
            "your patch would make source file 'src/a.rs' {lines} lines (currently 495, cap 500); \
             the ENTIRE patch is rejected"
        )))
    }
}

fn shrinking(rejections: usize, stuck: bool, budget: Option<Duration>) -> Shrinking {
    Shrinking {
        attempts: AtomicUsize::new(0),
        rejections,
        stuck,
        budget,
    }
}

fn branch(root: &std::path::Path) -> WorktreeBranchExecution {
    WorktreeBranchExecution {
        id: "implement-1-0".into(),
        role: "coder".into(),
        input_hash: None,
        workspace_root: root.to_path_buf(),
        execution: WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "implement-1-0".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: Some(WorkflowV2WriteMode::Worktree),
                options: Default::default(),
            },
            input: serde_json::json!({}),
            depends_on: Vec::new(),
        },
        refresh: None,
        time_budget: BranchTimeBudget::CallTimeBudget,
    }
}

async fn run(
    dispatch: &Shrinking,
    branch: &WorktreeBranchExecution,
    store: &WorkflowV2ResultStore,
) -> WorkflowResult<WorkflowV2Result> {
    run_worktree_branch_agent(
        "implement",
        None,
        dispatch,
        store,
        WorkflowV2AgentAdapter::new(),
        branch,
        None,
    )
    .await
}

#[tokio::test]
async fn a_branch_keeps_re_asking_while_its_overshoot_shrinks() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let dispatch = shrinking(20, false, None);
    let result = run(&dispatch, &branch(temp.path()), &store).await.unwrap();
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(dispatch.attempts.load(Ordering::SeqCst), 21);
}

#[tokio::test]
async fn a_spent_total_time_budget_does_not_end_a_progressing_branch() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let dispatch = shrinking(3, false, Some(Duration::ZERO));
    let result = run(&dispatch, &branch(temp.path()), &store).await.unwrap();
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
}

fn running_run(project: &std::path::Path) -> (WorkflowStore, String, u64) {
    let store = WorkflowStore::project(project);
    let mut run = store
        .create_run(crate::WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.into(),
            name: "branch-stall".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();
    let generation = run.generation;
    (store, run.id, generation)
}

/// Round 3 (decision B): a branch that stops converging never pauses the
/// run. Its outcome carries the stall, so the wave captures its work and
/// remediation runs; only remediation that makes no progress pauses.
#[tokio::test]
async fn a_branch_that_stops_converging_reports_its_outcome_and_never_pauses() {
    let temp = tempfile::tempdir().unwrap();
    let (workflows, run_id, generation) = running_run(temp.path());
    let store = WorkflowV2ResultStore::new(workflows.run_dir(&run_id).join("v2"));
    let dispatch = shrinking(usize::MAX, true, None);
    // The branch's own outcome: a stamped result, or the rejection the
    // wave records as the branch's failure (capturing its work). Never a
    // control error that would unwind the wave.
    match run(&dispatch, &branch(temp.path()), &store).await {
        Ok(result) => {
            assert_ne!(result.status, WorkflowV2Status::Accepted);
            assert_eq!(result.data["branch_no_progress"], true, "{result:#?}");
        }
        Err(error) => assert!(
            !matches!(
                error,
                WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
            ),
            "{error:?}"
        ),
    }
    let state = workflows.load_state(&run_id).unwrap();
    assert_eq!(state.status, RunStatus::Running);
    assert_eq!(state.generation, generation);
    assert_eq!(
        dispatch.attempts.load(Ordering::SeqCst),
        2,
        "one re-ask, then no progress"
    );
}

/// Writes one more file into the worktree, then loses the provider
/// connection, `drops` times; then accepts.
struct WritesThenDrops {
    attempts: AtomicUsize,
    drops: usize,
    writes: bool,
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for WritesThenDrops {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        _task: &str,
        root: Option<String>,
        _: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n >= self.drops {
            return Ok(WorkflowV2Result::accepted("done"));
        }
        if self.writes {
            let root = std::path::PathBuf::from(root.expect("worktree root"));
            std::fs::write(root.join(format!("step-{n}.txt")), "more\n").unwrap();
        }
        Err(WorkflowError::port("response_failed: connection reset"))
    }
}

fn git_worktree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@example.invalid"],
        vec!["config", "user.name", "t"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        crate::write_coordinator::worktree_isolation::run_git(&args, temp.path()).unwrap();
    }
    temp
}

/// Round 3 (decision C): a dropped connection after the session changed the
/// worktree is progress: the drop streak starts again, so a branch whose
/// every session advances the work is never stopped by drops.
#[tokio::test]
async fn transport_drops_after_worktree_progress_keep_re_asking() {
    let worktree = git_worktree();
    let side = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(side.path().join("v2"));
    let dispatch = WritesThenDrops {
        attempts: AtomicUsize::new(0),
        drops: crate::v2::transport_retry::MAX_TRANSPORT_RETRIES + 4,
        writes: true,
    };
    let result = run_worktree_branch_agent(
        "implement",
        None,
        &dispatch,
        &store,
        WorkflowV2AgentAdapter::new(),
        &branch(worktree.path()),
        None,
    )
    .await;
    let result = result.expect("an outcome");
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
}

/// Drops with no change to the worktree are no progress: the streak ends at
/// its bound.
#[tokio::test]
async fn transport_drops_without_worktree_progress_stop_at_the_bound() {
    let worktree = git_worktree();
    let side = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(side.path().join("v2"));
    let dispatch = WritesThenDrops {
        attempts: AtomicUsize::new(0),
        drops: usize::MAX,
        writes: false,
    };
    let _ = run_worktree_branch_agent(
        "implement",
        None,
        &dispatch,
        &store,
        WorkflowV2AgentAdapter::new(),
        &branch(worktree.path()),
        None,
    )
    .await;
    assert_eq!(
        dispatch.attempts.load(Ordering::SeqCst),
        crate::v2::transport_retry::MAX_TRANSPORT_RETRIES + 1
    );
}
