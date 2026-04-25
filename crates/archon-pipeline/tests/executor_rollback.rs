//! Integration tests for rollback engine and on_failure policy dispatch.
//!
//! Tests the four on_failure scenarios: Rollback, Fail, Skip, and partial
//! output cleanup between retries.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

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
// Mock TaskService — reusable across all rollback tests
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

/// Mock task service with per-agent call counting and configurable failure.
struct RollbackMockTaskService {
    responses: std::sync::Mutex<HashMap<String, MockResponse>>,
    submissions: std::sync::Mutex<Vec<SubmitRecord>>,
    agent_call_counts: std::sync::Mutex<HashMap<String, AtomicU32>>,
}

impl RollbackMockTaskService {
    fn new(responses: HashMap<String, MockResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
            submissions: std::sync::Mutex::new(Vec::new()),
            agent_call_counts: std::sync::Mutex::new(HashMap::new()),
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
impl TaskService for RollbackMockTaskService {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        let task_id = TaskId::new();
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
// Mock that writes a checkpoint on submit (for partial output cleanup test)
// ---------------------------------------------------------------------------

/// A mock that, on the first call for a given agent, succeeds at submit but
/// produces a checkpoint file before reporting the task as Failed at status.
/// This simulates the scenario where a step partially completes (checkpoint
/// written) and then fails at the status-poll phase.
struct CheckpointWritingMockService {
    store: Arc<PipelineStateStore>,
    responses: std::sync::Mutex<HashMap<String, MockResponse>>,
    submissions: std::sync::Mutex<Vec<SubmitRecord>>,
    agent_call_counts: std::sync::Mutex<HashMap<String, AtomicU32>>,
}

impl CheckpointWritingMockService {
    fn new(store: Arc<PipelineStateStore>, responses: HashMap<String, MockResponse>) -> Self {
        Self {
            store,
            responses: std::sync::Mutex::new(responses),
            submissions: std::sync::Mutex::new(Vec::new()),
            agent_call_counts: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl TaskService for CheckpointWritingMockService {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        let task_id = TaskId::new();
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

        // On the first (failing) call, write a checkpoint to simulate partial
        // output that should be cleaned up between retries.
        if should_fail && task_call_number == 1 {
            // We need the pipeline ID. Look it up from the store.
            let runs = self.store.list_runs().unwrap_or_default();
            if let Some(pid) = runs.first() {
                let _ = self
                    .store
                    .write_checkpoint(*pid, &agent_name, &json!({"partial": true}));
            }
        }

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

/// Check whether a checkpoint file exists for a given step.
fn checkpoint_exists(
    store_root: &std::path::Path,
    id: archon_pipeline::PipelineId,
    step_id: &str,
) -> bool {
    store_root
        .join(id.to_string())
        .join("checkpoints")
        .join(format!("{step_id}.json"))
        .exists()
}

fn make_step(
    id: &str,
    agent: &str,
    deps: Vec<&str>,
    on_failure: OnFailurePolicy,
    max_attempts: u32,
    base_delay_ms: u64,
) -> StepSpec {
    StepSpec {
        id: id.to_string(),
        agent: agent.to_string(),
        input: json!({}),
        depends_on: deps.into_iter().map(|d| d.to_string()).collect(),
        retry: RetrySpec {
            max_attempts,
            backoff: BackoffKind::Fixed,
            base_delay_ms,
        },
        timeout_secs: 1800,
        condition: None,
        on_failure,
    }
}

fn make_pipeline(steps: Vec<StepSpec>, max_parallelism: u32) -> PipelineSpec {
    PipelineSpec {
        name: "rollback-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 3600,
        max_parallelism,
        steps,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A -> B -> C pipeline, B fails (retries exhausted), `on_failure = Rollback`.
/// Assert: audit.log preserved, contains RolledBack events for A (the only
/// completed step), state.json deleted, checkpoints directory empty/deleted.
#[tokio::test]
async fn rollback_on_failure_default() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "agent-a".to_string(),
        MockResponse {
            output: json!({"result": "a-done"}),
            fail_first_n: 0,
        },
    );
    responses.insert(
        "agent-b".to_string(),
        MockResponse {
            output: json!(null),
            fail_first_n: 100, // always fails
        },
    );
    responses.insert(
        "agent-c".to_string(),
        MockResponse {
            output: json!({"result": "c-done"}),
            fail_first_n: 0,
        },
    );

    let mock = Arc::new(RollbackMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_pipeline(
        vec![
            make_step("A", "agent-a", vec![], OnFailurePolicy::Rollback, 1, 50),
            make_step("B", "agent-b", vec!["A"], OnFailurePolicy::Rollback, 1, 50),
            make_step("C", "agent-c", vec!["B"], OnFailurePolicy::Rollback, 1, 50),
        ],
        1,
    );

    let err = executor.run(spec).await.expect_err("pipeline should fail");
    assert!(
        matches!(err, archon_pipeline::PipelineError::StepFailed { .. }),
        "expected StepFailed, got: {err:?}"
    );

    // Get the pipeline run ID.
    let runs = store.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let id = runs[0];

    // audit.log should be preserved after rollback.
    let audit_path = tmp.path().join(id.to_string()).join("audit.log");
    assert!(
        audit_path.exists(),
        "audit.log should be preserved after rollback"
    );

    // state.json should be deleted after rollback.
    let state_path = tmp.path().join(id.to_string()).join("state.json");
    assert!(
        !state_path.exists(),
        "state.json should be deleted after rollback"
    );

    // Checkpoints directory should not exist (or be empty).
    let checkpoints_dir = tmp.path().join(id.to_string()).join("checkpoints");
    assert!(
        !checkpoints_dir.exists(),
        "checkpoints directory should be deleted after rollback"
    );

    // Audit log should contain RolledBack event for step A.
    let audit = read_audit_lines(tmp.path(), id);
    let rolled_back_events: Vec<&serde_json::Value> = audit
        .iter()
        .filter(|e| e["type"] == "rolled_back")
        .collect();
    assert_eq!(
        rolled_back_events.len(),
        1,
        "expected 1 RolledBack audit event (for A), got {}",
        rolled_back_events.len()
    );
    assert_eq!(rolled_back_events[0]["step"], "A");

    // C should never have been submitted.
    assert_eq!(mock.call_count_for("agent-c"), 0);
}

/// Same A -> B -> C pipeline, B fails, `on_failure = Fail`.
/// Assert: state.json preserved, PipelineState::Failed, checkpoints for A
/// preserved, NO RolledBack events in audit log.
#[tokio::test]
async fn fail_preserves_state() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "agent-a".to_string(),
        MockResponse {
            output: json!({"result": "a-done"}),
            fail_first_n: 0,
        },
    );
    responses.insert(
        "agent-b".to_string(),
        MockResponse {
            output: json!(null),
            fail_first_n: 100, // always fails
        },
    );
    responses.insert(
        "agent-c".to_string(),
        MockResponse {
            output: json!({"result": "c-done"}),
            fail_first_n: 0,
        },
    );

    let mock = Arc::new(RollbackMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_pipeline(
        vec![
            make_step("A", "agent-a", vec![], OnFailurePolicy::Fail, 1, 50),
            make_step("B", "agent-b", vec!["A"], OnFailurePolicy::Fail, 1, 50),
            make_step("C", "agent-c", vec!["B"], OnFailurePolicy::Fail, 1, 50),
        ],
        1,
    );

    let err = executor.run(spec).await.expect_err("pipeline should fail");
    assert!(
        matches!(err, archon_pipeline::PipelineError::StepFailed { .. }),
        "expected StepFailed, got: {err:?}"
    );

    let runs = store.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let id = runs[0];

    // state.json should be preserved.
    let run = store
        .load_state(id)
        .expect("state should still be loadable");
    assert_eq!(run.state, PipelineState::Failed);

    // A should be Finished, B Failed, C Pending.
    assert_eq!(run.steps["A"].state, StepRunState::Finished);
    assert_eq!(run.steps["B"].state, StepRunState::Failed);
    assert_eq!(run.steps["C"].state, StepRunState::Pending);

    // Checkpoint for A should be preserved.
    assert!(
        checkpoint_exists(tmp.path(), id, "A"),
        "checkpoint for A should be preserved under Fail policy"
    );

    // No RolledBack events in audit log.
    let audit = read_audit_lines(tmp.path(), id);
    let rolled_back_count = audit.iter().filter(|e| e["type"] == "rolled_back").count();
    assert_eq!(
        rolled_back_count, 0,
        "should have NO RolledBack events under Fail policy"
    );
}

/// 3 independent steps (no deps), middle one fails, `on_failure = Skip`.
/// Assert: failed step marked Skipped, other steps Finished, pipeline ends
/// Finished, audit has StepSkipped event.
#[tokio::test]
async fn skip_continues_pipeline() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert(
        "agent-a".to_string(),
        MockResponse {
            output: json!({"result": "a-done"}),
            fail_first_n: 0,
        },
    );
    responses.insert(
        "agent-b".to_string(),
        MockResponse {
            output: json!(null),
            fail_first_n: 100, // always fails
        },
    );
    responses.insert(
        "agent-c".to_string(),
        MockResponse {
            output: json!({"result": "c-done"}),
            fail_first_n: 0,
        },
    );

    let mock = Arc::new(RollbackMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    // All 3 steps are independent (no deps) so they run in a single level.
    let spec = make_pipeline(
        vec![
            make_step("A", "agent-a", vec![], OnFailurePolicy::Skip, 1, 50),
            make_step("B", "agent-b", vec![], OnFailurePolicy::Skip, 1, 50),
            make_step("C", "agent-c", vec![], OnFailurePolicy::Skip, 1, 50),
        ],
        3, // all 3 run concurrently
    );

    let id = executor
        .run(spec)
        .await
        .expect("pipeline should succeed (skip policy)");

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);

    // A and C should be Finished.
    assert_eq!(run.steps["A"].state, StepRunState::Finished);
    assert_eq!(run.steps["C"].state, StepRunState::Finished);

    // B should be Skipped.
    assert_eq!(run.steps["B"].state, StepRunState::Skipped);
    assert!(run.steps["B"].last_error.is_some());

    // Audit log should contain a StepSkipped event for B.
    let audit = read_audit_lines(tmp.path(), id);
    let skip_events: Vec<&serde_json::Value> = audit
        .iter()
        .filter(|e| e["type"] == "step_skipped")
        .collect();
    assert_eq!(skip_events.len(), 1, "expected 1 StepSkipped event");
    assert_eq!(skip_events[0]["step"], "B");

    // All 3 agents should have been submitted.
    assert_eq!(mock.call_count_for("agent-a"), 1);
    assert_eq!(mock.call_count_for("agent-b"), 1);
    assert_eq!(mock.call_count_for("agent-c"), 1);
}

/// Step with max_attempts=2: on first attempt it succeeds at submit but
/// fails at status (and the mock writes a checkpoint to simulate partial
/// output). Assert checkpoint is deleted before retry attempt 2 starts.
#[tokio::test]
async fn partial_output_cleaned_between_retries() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    // The step's agent name doubles as the step_id for the checkpoint mock.
    // We use step_id = "A" and agent = "A" to simplify the mock's checkpoint
    // writing (it uses agent_name as the step key).
    responses.insert(
        "A".to_string(),
        MockResponse {
            output: json!({"final": "result"}),
            fail_first_n: 1, // fails first call, succeeds second
        },
    );

    let mock = Arc::new(CheckpointWritingMockService::new(store.clone(), responses));
    let executor = PipelineExecutor::new(store.clone(), mock);

    let spec = make_pipeline(
        vec![make_step("A", "A", vec![], OnFailurePolicy::Fail, 2, 50)],
        1,
    );

    let id = executor
        .run(spec)
        .await
        .expect("pipeline should succeed on second attempt");

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);
    assert_eq!(run.steps["A"].state, StepRunState::Finished);
    assert_eq!(run.steps["A"].attempts, 2);

    // The checkpoint that exists now should be the final successful one,
    // NOT the partial one from the first attempt.
    let checkpoint = store
        .load_checkpoint(id, "A")
        .expect("checkpoint load should succeed")
        .expect("checkpoint should exist");
    assert_eq!(
        checkpoint,
        json!({"final": "result"}),
        "checkpoint should contain final result, not partial output"
    );

    // Verify via audit that a retry was scheduled.
    let audit = read_audit_lines(tmp.path(), id);
    let retry_count = audit
        .iter()
        .filter(|e| e["type"] == "retry_scheduled")
        .count();
    assert_eq!(retry_count, 1, "expected 1 RetryScheduled event");
}
