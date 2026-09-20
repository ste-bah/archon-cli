//! A session restarted mid-attempt is told what its worktree holds.
//!
//! The branch task is rendered once, against a clean worktree. When the
//! transport drops or the host cuts the session, the re-ask loop starts a
//! fresh session — and used to send it that same clean-worktree text. Live,
//! the restarted coder found its eight in-progress files only because it
//! happened to run `git status`.
use super::*;
use crate::WorkflowV2HostMethod;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn sh(args: &[&str], cwd: &Path) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn sealed_worktree(root: &Path) -> PathBuf {
    let canonical = root.join("repo");
    std::fs::create_dir_all(canonical.join("src")).unwrap();
    sh(&["init", "-q"], &canonical);
    sh(&["config", "user.email", "t@example.invalid"], &canonical);
    sh(&["config", "user.name", "t"], &canonical);
    std::fs::write(canonical.join("src/lib.rs"), "fn a() {}\n").unwrap();
    sh(&["add", "."], &canonical);
    sh(&["commit", "-qm", "base"], &canonical);
    let ws = root.join("ws");
    sh(
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            ws.to_str().unwrap(),
            "HEAD",
        ],
        &canonical,
    );
    ws
}

/// Fails the first call the way an empty reply that exhausted its repair
/// reaches the branch loop, then records the task text of the re-ask.
struct EmptyThenRecord {
    calls: AtomicUsize,
    tasks: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for EmptyThenRecord {
    fn call_time_budget(&self) -> Option<Duration> {
        Some(Duration::from_secs(14_400))
    }
    fn dispatch_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(7_200))
    }
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        _task: &str,
        _: Option<String>,
        execution: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        self.tasks
            .lock()
            .unwrap()
            .push(execution.call.options.task.clone().unwrap());
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(WorkflowError::StageFailed(format!(
                "agent transport failed: execution failed during bounded retries: root={}; last={}",
                crate::v2::WorkflowV2AgentError::EmptyReply,
                crate::v2::WorkflowV2AgentError::EmptyReply,
            )));
        }
        Ok(WorkflowV2Result::accepted("continued"))
    }
}

#[tokio::test]
async fn a_fresh_session_mid_attempt_is_told_its_own_partial_work_and_true_budget() {
    let temp = tempfile::tempdir().unwrap();
    let ws = sealed_worktree(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let dispatch = EmptyThenRecord {
        calls: AtomicUsize::new(0),
        tasks: Mutex::new(Vec::new()),
    };
    let base = "implement the module";
    let first_task = super::super::partial_work::with_host_preamble(
        base,
        super::super::partial_work::effective_call_budget(
            dispatch.dispatch_timeout(),
            dispatch.call_time_budget(),
            Duration::ZERO,
        ),
        None,
        &Default::default(),
    );
    let mut options = crate::v2::host_api::WorkflowV2HostOptions::default();
    options.task = Some(first_task);
    let branch = WorktreeBranchExecution {
        id: "agents-2-0".into(),
        role: "coder".into(),
        input_hash: None,
        workspace_root: ws.clone(),
        execution: WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "agents-2-0".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: Some(WorkflowV2WriteMode::Worktree),
                options,
            },
            input: serde_json::json!({}),
            depends_on: Vec::new(),
        },
        refresh: Some(super::super::partial_work::BranchTaskRefresh {
            base_task: base.to_string(),
            task_ids: vec!["TASK-001".to_string()],
            run_root: temp.path().join("run"),
            stage_id: "agents-2".into(),
            item_id: "agents-2-0".into(),
        }),
        time_budget: BranchTimeBudget::CallTimeBudget,
    };
    // The agent wrote two files under its ownership before the session ended.
    std::fs::write(ws.join("src/lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    std::fs::write(ws.join("src/new.rs"), "pub fn c() {}\n").unwrap();
    // ...and had one call refused by the guard, which recorded it in the
    // branch's sidecar as the guard does (Obs-8).
    let sidecar = crate::v2::write_read_set::path(&store, "agents-2-0");
    std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    std::fs::write(
        &sidecar,
        concat!(
            r#"{"kind":"refusal","call":7,"tool":"Bash","head":"cargo build --release","reason":"Release builds are disabled for this write-capable workflow call."}"#,
            "\n",
            r#"{"kind":"tool_call","call":7,"tool":"Bash","head":"cargo build --release","status":"refused: Release builds are disabled for this write-capable workflow call."}"#,
            "\n",
            r#"{"kind":"tool_call","call":8,"tool":"Bash","head":"git archive HEAD | tar -x -C /tmp/base","status":"exit 0"}"#,
            "\n",
        ),
    )
    .unwrap();

    run_worktree_branch_agent(
        "implement",
        None,
        &dispatch,
        &store,
        WorkflowV2AgentAdapter::new(),
        &branch,
        None,
    )
    .await
    .unwrap();

    let tasks = dispatch.tasks.lock().unwrap();
    assert_eq!(tasks.len(), 2, "one failed call and one re-ask");
    // (A) both sessions are told the limit the host actually applies.
    assert!(
        tasks[0].contains("this call has 120 minutes"),
        "{}",
        tasks[0]
    );
    assert!(!tasks[0].contains("240 minutes"));
    assert!(
        !tasks[0].contains("uncommitted work"),
        "first session started clean"
    );
    // (B) the restarted session is told about its own work, as its own.
    let second = &tasks[1];
    assert!(
        second.contains("Its uncommitted work (2 file(s)) has been applied to this workspace"),
        "{second}"
    );
    assert!(second.contains("src/lib.rs, src/new.rs"), "{second}");
    assert!(second.contains("same attempt, restarted"), "{second}");
    assert!(
        !second.contains("A previous attempt at this task"),
        "{second}"
    );
    assert!(second.contains("this call has 120 minutes"), "{second}");
    assert!(second.ends_with(base), "{second}");
    // (C) ...and what the ended session was refused and last ran.
    assert!(
        second.contains("The previous session had these tool calls refused by the host — do not retry them:\n  - Bash `cargo build --release` → Release builds are disabled for this write-capable workflow call."),
        "{second}"
    );
    assert!(
        second.contains("Its last 2 tool calls (most recent last) were:\n  - Bash `cargo build --release` → refused: Release builds are disabled for this write-capable workflow call.\n  - Bash `git archive HEAD | tar -x -C /tmp/base` → exit 0"),
        "{second}"
    );
    assert!(
        !tasks[0].contains("refused by the host"),
        "first session has no memory yet"
    );
}

/// The refresh is a write-branch concern: a branch without one re-asks with
/// the text it had, exactly as before.
#[tokio::test]
async fn a_branch_without_a_refresh_re_asks_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let dispatch = EmptyThenRecord {
        calls: AtomicUsize::new(0),
        tasks: Mutex::new(Vec::new()),
    };
    let mut options = crate::v2::host_api::WorkflowV2HostOptions::default();
    options.task = Some("verify the module".into());
    let branch = WorktreeBranchExecution {
        id: "verify-1-0".into(),
        role: "verifier".into(),
        input_hash: None,
        workspace_root: temp.path().to_path_buf(),
        execution: WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "verify-1-0".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: Some(WorkflowV2WriteMode::Worktree),
                options,
            },
            input: serde_json::json!({}),
            depends_on: Vec::new(),
        },
        refresh: None,
        time_budget: BranchTimeBudget::CallTimeBudget,
    };
    run_worktree_branch_agent(
        "verify",
        None,
        &dispatch,
        &store,
        WorkflowV2AgentAdapter::new(),
        &branch,
        None,
    )
    .await
    .unwrap();
    let tasks = dispatch.tasks.lock().unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0], tasks[1]);
    assert_eq!(tasks[1], "verify the module");
}

/// Returns the host's typed cut on every call and counts the calls.
struct AlwaysCut {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for AlwaysCut {
    fn call_time_budget(&self) -> Option<Duration> {
        Some(Duration::from_secs(14_400))
    }
    fn dispatch_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(1_800))
    }
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(WorkflowError::HostCallTimeout(
            "agent transport failed: subagent timed out after 1800s".to_string(),
        ))
    }
}

/// Issue-10 at the loop: the host's own cut, with hours of call budget still
/// unspent, is returned as the interrupted result after ONE dispatch. The
/// caller's retry-once / stall logic decides what happens next, not this loop.
#[tokio::test]
async fn a_host_cut_is_not_re_asked_by_the_loop() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let dispatch = AlwaysCut {
        calls: AtomicUsize::new(0),
    };
    let mut options = crate::v2::host_api::WorkflowV2HostOptions::default();
    options.task = Some("implement the module".into());
    let branch = WorktreeBranchExecution {
        id: "agents-4-0".into(),
        role: "coder".into(),
        input_hash: None,
        workspace_root: temp.path().to_path_buf(),
        execution: WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "agents-4-0".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: Some(WorkflowV2WriteMode::Worktree),
                options,
            },
            input: serde_json::json!({}),
            depends_on: Vec::new(),
        },
        refresh: None,
        time_budget: BranchTimeBudget::Fixed(Some(Duration::from_secs(1_800))),
    };
    let result = run_worktree_branch_agent(
        "implement",
        None,
        &dispatch,
        &store,
        WorkflowV2AgentAdapter::new(),
        &branch,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        dispatch.calls.load(Ordering::SeqCst),
        1,
        "no re-ask after a host cut"
    );
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data["branch_runtime_timeout"], true, "{result:#?}");
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "write_branch_timeout_agents-4-0"),
        "{result:#?}"
    );
}
