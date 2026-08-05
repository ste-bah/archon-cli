//! Integration tests for parallel dispatch within a DAG level.
//!
//! Uses a configurable mock [`TaskService`] with per-agent delays to
//! verify concurrency behaviour of [`PipelineExecutor`].

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
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
// Mock TaskService with configurable delays
// ---------------------------------------------------------------------------

/// Per-agent response configuration.
#[derive(Clone)]
struct MockResponse {
    output: serde_json::Value,
    should_fail: bool,
    delay_ms: u64,
}

/// Recorded call from submit().
#[derive(Debug, Clone)]
struct SubmitRecord {
    agent_name: String,
    task_id: TaskId,
}

/// Mock task service with configurable per-agent delays and concurrency tracking.
struct MockTaskService {
    responses: std::sync::Mutex<HashMap<String, MockResponse>>,
    submissions: std::sync::Mutex<Vec<SubmitRecord>>,
    /// Tracks the current number of in-flight tasks (between submit and status returning terminal).
    in_flight: AtomicU32,
    /// High-water mark for concurrent in-flight tasks.
    max_concurrent: AtomicU32,
}

impl MockTaskService {
    fn new(responses: HashMap<String, MockResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
            submissions: std::sync::Mutex::new(Vec::new()),
            in_flight: AtomicU32::new(0),
            max_concurrent: AtomicU32::new(0),
        }
    }

    fn max_concurrent(&self) -> u32 {
        self.max_concurrent.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TaskService for MockTaskService {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        let task_id = TaskId::new();
        self.submissions.lock().unwrap().push(SubmitRecord {
            agent_name: req.agent_name,
            task_id,
        });
        // Increment in-flight counter and update high-water mark.
        let prev = self.in_flight.fetch_add(1, Ordering::SeqCst);
        let current = prev + 1;
        self.max_concurrent.fetch_max(current, Ordering::SeqCst);
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
        // Both MutexGuards dropped here.

        // Simulate work by sleeping for the configured delay.
        if resp.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(resp.delay_ms)).await;
        }

        // Decrement in-flight counter now that the task is terminal.
        self.in_flight.fetch_sub(1, Ordering::SeqCst);

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

fn make_independent_steps(
    count: usize,
    delay_ms: u64,
    max_parallelism: u32,
) -> (PipelineSpec, HashMap<String, MockResponse>) {
    let mut steps = Vec::new();
    let mut responses = HashMap::new();

    for i in 0..count {
        let id = format!("S{}", i);
        let agent = format!("agent-{}", i);

        steps.push(StepSpec {
            id,
            agent: agent.clone(),
            input: json!({}),
            depends_on: vec![],
            retry: RetrySpec {
                max_attempts: 1,
                backoff: BackoffKind::Exponential,
                base_delay_ms: 1000,
            },
            timeout_secs: 1800,
            condition: None,
            on_failure: OnFailurePolicy::Fail,
        });

        responses.insert(
            agent,
            MockResponse {
                output: json!({"step": i}),
                should_fail: false,
                delay_ms,
            },
        );
    }

    let spec = PipelineSpec {
        name: "parallel-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 3600,
        max_parallelism,
        steps,
    };

    (spec, responses)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 3 independent steps, each with ~200ms delay, max_parallelism = 5.
/// All 3 should run concurrently. Wall time should be < 500ms.
#[tokio::test]
async fn three_independent_parallel() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let (spec, responses) = make_independent_steps(3, 200, 5);
    let mock = Arc::new(MockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let id = executor.run(spec).await.expect("pipeline should succeed");

    // Asserted on the in-flight high-water mark rather than wall time. The
    // property under test is that all 3 steps were dispatched together; a
    // deadline says so only indirectly, and says it wrongly on a loaded CI
    // runner, where these two tests took 1.7s and 1.1s against 500ms and
    // 1200ms budgets and failed for being on a slow machine.
    assert_eq!(
        mock.max_concurrent(),
        3,
        "all 3 independent steps must be in flight at once"
    );

    // Verify pipeline state.
    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);

    for i in 0..3 {
        let step_id = format!("S{}", i);
        let step_run = run.steps.get(&step_id).expect("step should exist");
        assert_eq!(
            step_run.state,
            archon_pipeline::StepRunState::Finished,
            "step {} should be Finished",
            step_id
        );
    }
}

/// 5 independent steps, each with ~200ms delay, max_parallelism = 2.
/// Should take 3 waves (2+2+1). Wall time between 400ms and 1200ms.
#[tokio::test]
async fn five_steps_parallelism_two() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let (spec, responses) = make_independent_steps(5, 200, 2);
    let mock = Arc::new(MockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let id = executor.run(spec).await.expect("pipeline should succeed");

    // Both halves of the property, without a clock: the cap is respected, and
    // it is actually used rather than the steps running one at a time.
    assert_eq!(
        mock.max_concurrent(),
        2,
        "5 steps under max_parallelism = 2 must run exactly 2 at a time"
    );

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);
}

/// 10 independent steps, max_parallelism = 5.
/// Uses an AtomicU32 counter to track concurrent in-flight tasks.
/// Assert max concurrent never exceeds 5.
#[tokio::test]
async fn concurrency_cap_enforced() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let (spec, responses) = make_independent_steps(10, 100, 5);
    let mock = Arc::new(MockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let id = executor.run(spec).await.expect("pipeline should succeed");

    // The mock tracks max concurrent in-flight tasks.
    let max_conc = mock.max_concurrent();
    assert!(
        max_conc <= 5,
        "max concurrent tasks should be <= 5, got {}",
        max_conc
    );

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);
}

/// 3 independent steps, each with ~200ms delay, max_parallelism = 1.
/// Should be sequential. Wall time >= 500ms.
#[tokio::test]
async fn parallelism_one_is_sequential() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let (spec, responses) = make_independent_steps(3, 200, 1);
    let mock = Arc::new(MockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let id = executor.run(spec).await.expect("pipeline should succeed");

    assert_eq!(
        mock.max_concurrent(),
        1,
        "max_parallelism = 1 must never have two steps in flight"
    );

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);
}
