//! Integration tests for retry with exponential backoff in the pipeline executor.
//!
//! Uses a configurable mock [`TaskService`] that can fail the first N calls
//! per agent before succeeding, to verify retry behaviour of
//! [`PipelineExecutor`].

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    PipelineStateStore, RetrySpec, StepRunState, StepSpec,
};

// ---------------------------------------------------------------------------
// Mock TaskService with per-agent call counting and configurable failure
// ---------------------------------------------------------------------------

/// Per-agent response configuration.
#[derive(Clone)]
struct MockResponse {
    output: serde_json::Value,
    /// Fail the first N submissions for this agent, then succeed.
    fail_first_n: u32,
}

/// Recorded call from submit().
#[derive(Debug, Clone)]
struct SubmitRecord {
    agent_name: String,
    task_id: TaskId,
}

/// Mock task service that tracks per-agent call counts and can fail the first
/// N calls for each agent.
struct RetryMockTaskService {
    responses: std::sync::Mutex<HashMap<String, MockResponse>>,
    submissions: std::sync::Mutex<Vec<SubmitRecord>>,
    /// Per-agent call counter (keyed by agent_name).
    agent_call_counts: std::sync::Mutex<HashMap<String, AtomicU32>>,
}

impl RetryMockTaskService {
    fn new(responses: HashMap<String, MockResponse>) -> Self {
        let agent_call_counts = std::sync::Mutex::new(HashMap::new());
        Self {
            responses: std::sync::Mutex::new(responses),
            submissions: std::sync::Mutex::new(Vec::new()),
            agent_call_counts,
        }
    }

    /// Get the current call count for a given agent.
    fn call_count_for(&self, agent_name: &str) -> u32 {
        let counts = self.agent_call_counts.lock().unwrap();
        counts
            .get(agent_name)
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0)
    }
}

#[async_trait]
impl TaskService for RetryMockTaskService {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        let task_id = TaskId::new();

        // Increment per-agent call count.
        {
            let mut counts = self.agent_call_counts.lock().unwrap();
            counts
                .entry(req.agent_name.clone())
                .or_insert_with(|| AtomicU32::new(0))
                .fetch_add(1, Ordering::SeqCst);
        }

        self.submissions.lock().unwrap().push(SubmitRecord {
            agent_name: req.agent_name,
            task_id,
        });
        Ok(task_id)
    }

    async fn status(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
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

        // Determine the call number for THIS specific task_id.
        // We need to figure out which submission number this is for the agent.
        let task_call_number = {
            let submissions = self.submissions.lock().unwrap();
            let mut n = 0u32;
            for rec in submissions.iter() {
                if rec.agent_name == agent_name {
                    n += 1;
                    if rec.task_id == id {
                        break;
                    }
                }
            }
            n
        };

        let should_fail = task_call_number <= resp.fail_first_n;

        let state = if should_fail {
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
            error: if should_fail {
                Some("mock transient failure".to_string())
            } else {
                None
            },
        })
    }

    async fn result(&self, id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
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

        // Determine call number for this task.
        let task_call_number = {
            let submissions = self.submissions.lock().unwrap();
            let mut n = 0u32;
            for rec in submissions.iter() {
                if rec.agent_name == agent_name {
                    n += 1;
                    if rec.task_id == id {
                        break;
                    }
                }
            }
            n
        };

        if task_call_number <= resp.fail_first_n {
            return Err(TaskError::NotFound(id));
        }

        Ok(TaskResultStream::Inline(
            serde_json::to_string(&resp.output).unwrap(),
        ))
    }

    async fn cancel(&self, _id: TaskId) -> Result<(), TaskError> {
        Err(TaskError::Unimplemented)
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
fn read_audit_lines(
    store_root: &std::path::Path,
    id: archon_pipeline::PipelineId,
) -> Vec<serde_json::Value> {
    let audit_path = store_root.join(id.to_string()).join("audit.log");
    let raw = std::fs::read_to_string(&audit_path).unwrap_or_default();
    raw.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("valid audit JSON"))
        .collect()
}

fn make_retry_spec(
    id: &str,
    agent: &str,
    max_attempts: u32,
    backoff: BackoffKind,
    base_delay_ms: u64,
) -> StepSpec {
    StepSpec {
        id: id.to_string(),
        agent: agent.to_string(),
        input: json!({}),
        depends_on: vec![],
        retry: RetrySpec {
            max_attempts,
            backoff,
            base_delay_ms,
        },
        timeout_secs: 1800,
        condition: None,
        on_failure: OnFailurePolicy::Fail,
    }
}

fn make_pipeline(steps: Vec<StepSpec>) -> PipelineSpec {
    PipelineSpec {
        name: "retry-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 3600,
        max_parallelism: 1,
        steps,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Step fails twice then succeeds on attempt 3.
/// Assert: 3 StepStarted events, 2 RetryScheduled events, final state
/// Finished, step_run.attempts == 3.
#[tokio::test]
async fn retries_then_succeeds() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "agent-retry".to_string(),
        MockResponse {
            output: json!({"result": "ok"}),
            fail_first_n: 2,
        },
    );

    let mock = Arc::new(RetryMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_pipeline(vec![make_retry_spec(
        "A",
        "agent-retry",
        3,
        BackoffKind::Exponential,
        50,
    )]);

    let id = executor.run(spec).await.expect("pipeline should succeed after retries");

    // Verify final pipeline state.
    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);

    // Step should be Finished with 3 attempts.
    let step_run = run.steps.get("A").expect("step A should exist");
    assert_eq!(step_run.state, StepRunState::Finished);
    assert_eq!(step_run.attempts, 3, "should have taken 3 attempts");

    // 3 submissions total.
    assert_eq!(mock.call_count_for("agent-retry"), 3);

    // Audit: Started + 3x StepStarted + 2x RetryScheduled + StepFinished + Finished.
    let audit = read_audit_lines(tmp.path(), id);

    let step_started_count = audit.iter().filter(|e| e["type"] == "step_started").count();
    assert_eq!(step_started_count, 3, "expected 3 StepStarted audit events");

    let retry_scheduled_count = audit.iter().filter(|e| e["type"] == "retry_scheduled").count();
    assert_eq!(
        retry_scheduled_count, 2,
        "expected 2 RetryScheduled audit events"
    );

    let step_finished_count = audit.iter().filter(|e| e["type"] == "step_finished").count();
    assert_eq!(step_finished_count, 1, "expected 1 StepFinished audit event");
}

/// Step fails all 3 attempts — retries exhausted.
/// Assert: 3 StepStarted, 2 RetryScheduled, final StepFailed, run ends Failed.
#[tokio::test]
async fn retries_exhausted_then_fails() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "agent-fail".to_string(),
        MockResponse {
            output: json!(null),
            fail_first_n: 100, // always fails
        },
    );

    let mock = Arc::new(RetryMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_pipeline(vec![make_retry_spec(
        "A",
        "agent-fail",
        3,
        BackoffKind::Exponential,
        50,
    )]);

    let err = executor.run(spec).await.expect_err("pipeline should fail");
    assert!(
        matches!(err, archon_pipeline::PipelineError::StepFailed { .. }),
        "expected StepFailed, got: {err:?}"
    );

    // Load the persisted state.
    let runs = store.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let id = runs[0];

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Failed);

    let step_run = run.steps.get("A").expect("step A should exist");
    assert_eq!(step_run.state, StepRunState::Failed);
    assert!(step_run.last_error.is_some());

    // 3 submissions total.
    assert_eq!(mock.call_count_for("agent-fail"), 3);

    // Audit events.
    let audit = read_audit_lines(tmp.path(), id);

    let step_started_count = audit.iter().filter(|e| e["type"] == "step_started").count();
    assert_eq!(step_started_count, 3, "expected 3 StepStarted");

    let retry_scheduled_count = audit.iter().filter(|e| e["type"] == "retry_scheduled").count();
    assert_eq!(retry_scheduled_count, 2, "expected 2 RetryScheduled");

    let step_failed_count = audit.iter().filter(|e| e["type"] == "step_failed").count();
    assert_eq!(step_failed_count, 1, "expected 1 StepFailed");
}

/// Step with max_attempts = 1 fails immediately — no retries.
/// Assert: 1 StepStarted, 0 RetryScheduled, immediate StepFailed.
#[tokio::test]
async fn no_retry_on_default() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "agent-once".to_string(),
        MockResponse {
            output: json!(null),
            fail_first_n: 100, // always fails
        },
    );

    let mock = Arc::new(RetryMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_pipeline(vec![make_retry_spec(
        "A",
        "agent-once",
        1, // no retries
        BackoffKind::Exponential,
        50,
    )]);

    let err = executor.run(spec).await.expect_err("pipeline should fail");
    assert!(matches!(
        err,
        archon_pipeline::PipelineError::StepFailed { .. }
    ));

    let runs = store.list_runs().expect("list runs");
    let id = runs[0];

    // Only 1 submission.
    assert_eq!(mock.call_count_for("agent-once"), 1);

    // Audit events.
    let audit = read_audit_lines(tmp.path(), id);

    let step_started_count = audit.iter().filter(|e| e["type"] == "step_started").count();
    assert_eq!(step_started_count, 1, "expected 1 StepStarted");

    let retry_scheduled_count = audit.iter().filter(|e| e["type"] == "retry_scheduled").count();
    assert_eq!(retry_scheduled_count, 0, "expected 0 RetryScheduled");

    let step_failed_count = audit.iter().filter(|e| e["type"] == "step_failed").count();
    assert_eq!(step_failed_count, 1, "expected 1 StepFailed");
}

/// Step fails 3 times then succeeds on attempt 4 with base_delay_ms = 50.
/// Measure Instant between retries and assert delays are monotonically
/// increasing (exponential: 50ms, 100ms, 200ms within 20% tolerance).
#[tokio::test]
async fn exponential_delays_increase() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "agent-delay".to_string(),
        MockResponse {
            output: json!({"done": true}),
            fail_first_n: 3,
        },
    );

    let mock = Arc::new(RetryMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_pipeline(vec![make_retry_spec(
        "A",
        "agent-delay",
        4,
        BackoffKind::Exponential,
        50,
    )]);

    let start = Instant::now();
    let id = executor.run(spec).await.expect("pipeline should succeed on 4th attempt");
    let total_elapsed = start.elapsed();

    // Verify success.
    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);
    assert_eq!(run.steps["A"].attempts, 4);

    // Total time should be at least 50 + 100 + 200 = 350ms of backoff delays.
    // With tolerance, check >= 280ms (80% of 350ms).
    assert!(
        total_elapsed >= Duration::from_millis(280),
        "expected total elapsed >= 280ms (from backoff delays), got {:?}",
        total_elapsed
    );

    // Read audit events and extract RetryScheduled delay_ms values.
    let audit = read_audit_lines(tmp.path(), id);
    let retry_delays: Vec<u64> = audit
        .iter()
        .filter(|e| e["type"] == "retry_scheduled")
        .map(|e| e["delay_ms"].as_u64().expect("delay_ms should be u64"))
        .collect();

    assert_eq!(retry_delays.len(), 3, "expected 3 RetryScheduled events");

    // Expected delays: 50, 100, 200 (exponential with base 50ms).
    assert_eq!(retry_delays[0], 50, "first delay should be 50ms");
    assert_eq!(retry_delays[1], 100, "second delay should be 100ms");
    assert_eq!(retry_delays[2], 200, "third delay should be 200ms");

    // Verify delays are monotonically increasing.
    for i in 1..retry_delays.len() {
        assert!(
            retry_delays[i] > retry_delays[i - 1],
            "delays should be monotonically increasing: {:?}",
            retry_delays
        );
    }
}
