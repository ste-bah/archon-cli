//! Issue 263, round 2: a write branch is bounded by no progress, never by a
//! total count of re-asks or a total wall clock, and a branch that stops
//! progressing pauses the run (resumable, with evidence) instead of ending
//! its work as `NeedsReview`.

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

fn branch(root: &std::path::Path, pause: Option<BranchPause>) -> WorktreeBranchExecution {
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
        pause,
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
    let result = run(&dispatch, &branch(temp.path(), None), &store)
        .await
        .unwrap();
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(dispatch.attempts.load(Ordering::SeqCst), 21);
}

#[tokio::test]
async fn a_spent_total_time_budget_does_not_end_a_progressing_branch() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let dispatch = shrinking(3, false, Some(Duration::ZERO));
    let result = run(&dispatch, &branch(temp.path(), None), &store)
        .await
        .unwrap();
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

#[tokio::test]
async fn a_branch_that_stops_converging_pauses_the_run_with_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let (workflows, run_id, generation) = running_run(temp.path());
    let store = WorkflowV2ResultStore::new(workflows.run_dir(&run_id).join("v2"));
    let dispatch = shrinking(usize::MAX, true, None);
    let pause = BranchPause {
        store: workflows.clone(),
        run_id: run_id.clone(),
        generation,
    };
    let error = run(&dispatch, &branch(temp.path(), Some(pause)), &store)
        .await
        .expect_err("a stalled branch pauses the run");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    let state = workflows.load_state(&run_id).unwrap();
    assert_eq!(state.status, RunStatus::Paused);
    assert_eq!(state.generation, generation + 1);
    let events = std::fs::read_to_string(workflows.events_path(&run_id)).unwrap();
    assert!(events.contains("write_branch_stall_pause"), "{events}");
    assert_eq!(
        dispatch.attempts.load(Ordering::SeqCst),
        2,
        "one re-ask, then no progress"
    );
}

#[tokio::test]
async fn a_stale_branch_never_pauses_a_newer_generation() {
    let temp = tempfile::tempdir().unwrap();
    let (workflows, run_id, generation) = running_run(temp.path());
    let store = WorkflowV2ResultStore::new(workflows.run_dir(&run_id).join("v2"));
    // The operator paused and resumed while the branch ran.
    let mut state = workflows.load_state(&run_id).unwrap();
    state.generation = generation + 2;
    workflows.save_state(&state).unwrap();
    let pause = BranchPause {
        store: workflows.clone(),
        run_id: run_id.clone(),
        generation,
    };
    let error = run(
        &shrinking(usize::MAX, true, None),
        &branch(temp.path(), Some(pause)),
        &store,
    )
    .await
    .expect_err("the obsolete branch stops");
    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "{error:?}"
    );
    let state = workflows.load_state(&run_id).unwrap();
    assert_eq!(state.status, RunStatus::Running);
    assert_eq!(state.generation, generation + 2);
}
