//! Integration tests for global and per-step timeouts with graceful cancellation.
//!
//! Uses a configurable mock [`TaskService`] with per-agent delays and
//! cancel-tracking to verify timeout and cancellation behaviour of
//! [`PipelineExecutor`].

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::Stream;
use serde_json::json;
use tempfile::TempDir;

use archon_core::tasks::{
    SubmitRequest, TaskError, TaskEvent, TaskFilter, TaskId, TaskResultStream, TaskService,
    TaskSnapshot, TaskState,
};
use archon_pipeline::{
    BackoffKind, OnFailurePolicy, PipelineExecutor, PipelineSpec, PipelineState,
    PipelineStateStore, RetrySpec, StepSpec,
};

// ---------------------------------------------------------------------------
// Mock TaskService with configurable delays and cancel tracking
// ---------------------------------------------------------------------------

/// Per-agent response configuration.
#[derive(Clone)]
struct MockResponse {
    output: serde_json::Value,
    should_fail: bool,
    /// Delay in milliseconds applied on *each* call to `status()`.
    status_delay_ms: u64,
}

/// Recorded call from submit().
#[derive(Debug, Clone)]
struct SubmitRecord {
    agent_name: String,
    task_id: TaskId,
}

/// Per-task poll counter for thread-safe access.
struct PollTracker {
    counts: std::sync::Mutex<HashMap<TaskId, u32>>,
}

impl PollTracker {
    fn new() -> Self {
        Self {
            counts: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn increment(&self, id: TaskId) -> u32 {
        let mut map = self.counts.lock().unwrap();
        let entry = map.entry(id).or_insert(0);
        *entry += 1;
        *entry
    }
}

/// Enhanced mock that combines the response map with poll tracking.
struct DelayMockTaskService {
    responses: std::sync::Mutex<HashMap<String, MockResponse>>,
    submissions: std::sync::Mutex<Vec<SubmitRecord>>,
    cancel_count: AtomicU32,
    poll_tracker: PollTracker,
    /// If 0, the task never becomes terminal (simulates a hanging task).
    polls_before_terminal: u32,
}

impl DelayMockTaskService {
    fn new(
        responses: HashMap<String, MockResponse>,
        polls_before_terminal: u32,
    ) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
            submissions: std::sync::Mutex::new(Vec::new()),
            cancel_count: AtomicU32::new(0),
            poll_tracker: PollTracker::new(),
            polls_before_terminal,
        }
    }

    fn cancel_count(&self) -> u32 {
        self.cancel_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TaskService for DelayMockTaskService {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        let task_id = TaskId::new();
        self.submissions.lock().unwrap().push(SubmitRecord {
            agent_name: req.agent_name,
            task_id,
        });
        Ok(task_id)
    }

    async fn status(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        // Extract what we need from the locks, then drop them before any await.
        let (agent_name, resp) = {
            let submissions = self.submissions.lock().unwrap();
            let record = submissions
                .iter()
                .find(|r| r.task_id == id)
                .ok_or(TaskError::NotFound(id))?;
            let agent_name = record.agent_name.clone();

            let responses = self.responses.lock().unwrap();
            let resp = responses
                .get(&agent_name)
                .ok_or(TaskError::NotFound(id))?
                .clone();

            (agent_name, resp)
        };

        // Simulate work by sleeping for the configured delay on every poll.
        if resp.status_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(resp.status_delay_ms)).await;
        }

        // Track how many times we've been polled for this task.
        let poll_count = self.poll_tracker.increment(id);

        // Decide whether this poll returns a terminal state.
        let is_terminal = self.polls_before_terminal > 0
            && poll_count >= self.polls_before_terminal;

        if !is_terminal {
            // Return Running — the executor will poll again.
            return Ok(TaskSnapshot {
                id,
                agent_name,
                state: TaskState::Running,
                progress_pct: None,
                created_at: chrono::Utc::now(),
                started_at: Some(chrono::Utc::now()),
                finished_at: None,
                error: None,
            });
        }

        let state = if resp.should_fail {
            TaskState::Failed
        } else {
            TaskState::Finished
        };

        Ok(TaskSnapshot {
            id,
            agent_name,
            state,
            progress_pct: None,
            created_at: chrono::Utc::now(),
            started_at: Some(chrono::Utc::now()),
            finished_at: Some(chrono::Utc::now()),
            error: if resp.should_fail {
                Some("mock failure".to_string())
            } else {
                None
            },
        })
    }

    async fn result(&self, id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
        let submissions = self.submissions.lock().unwrap();
        let record = submissions
            .iter()
            .find(|r| r.task_id == id)
            .ok_or(TaskError::NotFound(id))?;

        let responses = self.responses.lock().unwrap();
        let resp = responses
            .get(&record.agent_name)
            .ok_or(TaskError::NotFound(id))?
            .clone();

        if resp.should_fail {
            return Err(TaskError::NotFound(id));
        }

        Ok(TaskResultStream::Inline(
            serde_json::to_string(&resp.output).unwrap(),
        ))
    }

    async fn cancel(&self, _id: TaskId) -> Result<(), TaskError> {
        self.cancel_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn subscribe_events(
        &self,
        _id: TaskId,
        _from_seq: u64,
    ) -> Result<Pin<Box<dyn Stream<Item = TaskEvent> + Send>>, TaskError> {
        Err(TaskError::Unimplemented)
    }

    async fn list(&self, _filter: TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
        Err(TaskError::Unimplemented)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read the audit log lines from disk for a given pipeline run.
fn read_audit_lines(store_root: &std::path::Path, id: archon_pipeline::PipelineId) -> Vec<serde_json::Value> {
    let audit_path = store_root.join(id.to_string()).join("audit.log");
    let raw = std::fs::read_to_string(&audit_path).unwrap_or_default();
    raw.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("valid audit JSON"))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Per-step timeout fires when a step's polling loop exceeds `timeout_secs`.
///
/// Setup: single step with `timeout_secs = 1` and a mock that never becomes
/// terminal (polls_before_terminal = 0) with a 200ms delay per status call.
/// After 1 second the per-step timeout fires, `cancel(task_id)` is called,
/// and the pipeline fails with `PipelineError::Timeout { step: Some("A") }`.
#[tokio::test]
async fn per_step_timeout_triggers_cancel() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "slow-agent".to_string(),
        MockResponse {
            output: json!({"ok": true}),
            should_fail: false,
            status_delay_ms: 200, // each poll takes 200ms
        },
    );

    // polls_before_terminal = 0 means the task never becomes terminal.
    let mock = Arc::new(DelayMockTaskService::new(responses, 0));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = PipelineSpec {
        name: "step-timeout-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 30, // large enough to not interfere
        max_parallelism: 1,
        steps: vec![StepSpec {
            id: "A".to_string(),
            agent: "slow-agent".to_string(),
            input: json!({}),
            depends_on: vec![],
            retry: RetrySpec {
                max_attempts: 1,
                backoff: BackoffKind::Exponential,
                base_delay_ms: 1000,
            },
            timeout_secs: 1, // 1 second per-step timeout
            condition: None,
            on_failure: OnFailurePolicy::Fail,
        }],
    };

    let err = executor.run(spec).await.expect_err("pipeline should fail with timeout");

    // Verify it's a Timeout error on step "A".
    match &err {
        archon_pipeline::PipelineError::Timeout { step } => {
            assert_eq!(step.as_deref(), Some("A"), "timeout should report step A");
        }
        other => panic!("expected PipelineError::Timeout, got: {other:?}"),
    }

    // Verify cancel was called on the task.
    assert!(
        mock.cancel_count() >= 1,
        "task_service.cancel() should have been called at least once, got {}",
        mock.cancel_count()
    );
}

/// Global timeout cancels the entire pipeline when execution exceeds
/// `global_timeout_secs`.
///
/// Setup: pipeline with `global_timeout_secs = 1` and one step that takes
/// 2+ seconds (never becomes terminal). Assert: run ends in Cancelled state,
/// audit log contains a Cancelled event.
#[tokio::test]
async fn global_timeout_cancels_pipeline() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "hanging-agent".to_string(),
        MockResponse {
            output: json!({"ok": true}),
            should_fail: false,
            status_delay_ms: 200,
        },
    );

    // Never becomes terminal.
    let mock = Arc::new(DelayMockTaskService::new(responses, 0));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = PipelineSpec {
        name: "global-timeout-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 1, // 1 second global timeout
        max_parallelism: 1,
        steps: vec![StepSpec {
            id: "A".to_string(),
            agent: "hanging-agent".to_string(),
            input: json!({}),
            depends_on: vec![],
            retry: RetrySpec {
                max_attempts: 1,
                backoff: BackoffKind::Exponential,
                base_delay_ms: 1000,
            },
            timeout_secs: 30, // large — should not fire before global
            condition: None,
            on_failure: OnFailurePolicy::Fail,
        }],
    };

    let err = executor.run(spec).await.expect_err("pipeline should fail with timeout");

    // Global timeout yields Timeout { step: None }.
    match &err {
        archon_pipeline::PipelineError::Timeout { step } => {
            assert!(step.is_none(), "global timeout should have step: None");
        }
        other => panic!("expected PipelineError::Timeout, got: {other:?}"),
    }

    // Verify the persisted run state is Cancelled.
    let runs = store.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let id = runs[0];
    let run = store.load_state(id).expect("state should load");
    assert_eq!(
        run.state,
        PipelineState::Cancelled,
        "run should be in Cancelled state"
    );

    // Audit log should contain a "cancelled" event.
    let audit = read_audit_lines(tmp.path(), id);
    let has_cancelled = audit.iter().any(|e| e["type"] == "cancelled");
    assert!(
        has_cancelled,
        "audit log should contain a cancelled event, got: {audit:?}"
    );
}

/// User-initiated cancel via `executor.cancel(id)` mid-execution.
///
/// Setup: 2-step linear pipeline (A -> B), each step takes ~500ms.
/// After 200ms, look up the active run and call `executor.cancel(id)`.
/// Assert: the run ends in Cancelled state.
#[tokio::test]
async fn user_cancel_mid_execution() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    // Each step: 500ms delay per poll, becomes terminal after 5 polls (~2.5s).
    responses.insert(
        "agent-a".to_string(),
        MockResponse {
            output: json!({"a": 1}),
            should_fail: false,
            status_delay_ms: 500,
        },
    );
    responses.insert(
        "agent-b".to_string(),
        MockResponse {
            output: json!({"b": 2}),
            should_fail: false,
            status_delay_ms: 500,
        },
    );

    // Becomes terminal after 5 polls (so ~2.5s per step, well over our cancel window).
    let mock = Arc::new(DelayMockTaskService::new(responses, 5));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = PipelineSpec {
        name: "cancel-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 60, // large — should not interfere
        max_parallelism: 1,
        steps: vec![
            StepSpec {
                id: "A".to_string(),
                agent: "agent-a".to_string(),
                input: json!({}),
                depends_on: vec![],
                retry: RetrySpec {
                    max_attempts: 1,
                    backoff: BackoffKind::Exponential,
                    base_delay_ms: 1000,
                },
                timeout_secs: 60, // large — should not interfere
                condition: None,
                on_failure: OnFailurePolicy::Fail,
            },
            StepSpec {
                id: "B".to_string(),
                agent: "agent-b".to_string(),
                input: json!({}),
                depends_on: vec!["A".to_string()],
                retry: RetrySpec {
                    max_attempts: 1,
                    backoff: BackoffKind::Exponential,
                    base_delay_ms: 1000,
                },
                timeout_secs: 60,
                condition: None,
                on_failure: OnFailurePolicy::Fail,
            },
        ],
    };

    let executor_clone = executor.clone();
    let store_clone = store.clone();

    // Spawn the pipeline execution in a background task.
    let handle = tokio::spawn(async move {
        executor_clone.run(spec).await
    });

    // Wait a moment for the run to be created and registered.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Find the active run.
    let runs = store_clone.list_runs().expect("list runs");
    assert!(!runs.is_empty(), "should have at least one run");
    let id = runs[0];

    // Cancel it.
    executor
        .cancel(id)
        .await
        .expect("cancel should succeed");

    // Wait for the spawned task to finish.
    let result = handle.await.expect("task should not panic");

    // The pipeline should have failed with a timeout/cancellation error.
    assert!(result.is_err(), "pipeline should have been cancelled");

    // Verify the persisted run state is Cancelled.
    let run = store_clone.load_state(id).expect("state should load");
    assert_eq!(
        run.state,
        PipelineState::Cancelled,
        "run should be in Cancelled state after user cancel"
    );

    // Audit log should contain a "cancelled" event.
    let audit = read_audit_lines(tmp.path(), id);
    let has_cancelled = audit.iter().any(|e| e["type"] == "cancelled");
    assert!(
        has_cancelled,
        "audit log should contain a cancelled event after user cancel, got: {audit:?}"
    );
}
