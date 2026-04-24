//! Integration tests for pipeline resume-from-checkpoint.
//!
//! Tests the `PipelineEngine::resume` method which allows a failed or
//! cancelled pipeline to be restarted from its last checkpoint, skipping
//! steps that have already finished.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::Stream;
use serde_json::json;
use tempfile::TempDir;

use archon_core::tasks::{
    SubmitRequest, TaskError, TaskEvent, TaskFilter, TaskId, TaskResultStream, TaskService,
    TaskSnapshot, TaskState,
};
use archon_pipeline::{
    BackoffKind, DefaultPipelineEngine, OnFailurePolicy, PipelineEngine, PipelineError,
    PipelineSpec, PipelineState, PipelineStateStore, RetrySpec, StepRunState, StepSpec,
};

// ---------------------------------------------------------------------------
// Mock TaskService — stateful, with per-agent call counting and
// configurable "fail until call N" behavior.
// ---------------------------------------------------------------------------

/// Recorded call from submit().
#[derive(Debug, Clone)]
struct SubmitRecord {
    agent_name: String,
    task_id: TaskId,
}

/// Mock task service that can fail specific agents on their first N calls
/// and succeed on subsequent calls.  Used to simulate first-run failure
/// followed by successful resume.
struct ResumeMockTaskService {
    /// Maps agent name to its output value on success.
    outputs: std::sync::Mutex<HashMap<String, serde_json::Value>>,
    /// All submit calls in order.
    submissions: std::sync::Mutex<Vec<SubmitRecord>>,
    /// Per-agent call counts (incremented on each submit).
    agent_call_counts: std::sync::Mutex<HashMap<String, AtomicU32>>,
    /// Agent name -> first N calls fail, call N+1 succeeds.
    fail_until: std::sync::Mutex<HashMap<String, u32>>,
}

impl ResumeMockTaskService {
    fn new(outputs: HashMap<String, serde_json::Value>, fail_until: HashMap<String, u32>) -> Self {
        Self {
            outputs: std::sync::Mutex::new(outputs),
            submissions: std::sync::Mutex::new(Vec::new()),
            agent_call_counts: std::sync::Mutex::new(HashMap::new()),
            fail_until: std::sync::Mutex::new(fail_until),
        }
    }

    /// Get the current call count for a given agent.
    fn call_count(&self, agent: &str) -> u32 {
        self.agent_call_counts
            .lock()
            .unwrap()
            .get(agent)
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0)
    }
}

#[async_trait]
impl TaskService for ResumeMockTaskService {
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
        let (agent_name, task_call_number) = {
            let submissions = self.submissions.lock().unwrap();
            let record = submissions
                .iter()
                .find(|r| r.task_id == id)
                .ok_or(TaskError::NotFound(id))?;
            let agent_name = record.agent_name.clone();

            // Determine which call number this specific task_id corresponds to.
            let mut n = 0u32;
            for rec in submissions.iter() {
                if rec.agent_name == agent_name {
                    n += 1;
                    if rec.task_id == id {
                        break;
                    }
                }
            }
            (agent_name, n)
        };

        let fail_threshold = self
            .fail_until
            .lock()
            .unwrap()
            .get(&agent_name)
            .copied()
            .unwrap_or(0);

        let should_fail = task_call_number <= fail_threshold;

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
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
            error: if should_fail {
                Some("mock failure".to_string())
            } else {
                None
            },
        })
    }

    async fn result(&self, id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
        let agent_name = {
            let submissions = self.submissions.lock().unwrap();
            let record = submissions
                .iter()
                .find(|r| r.task_id == id)
                .ok_or(TaskError::NotFound(id))?;
            record.agent_name.clone()
        };

        let outputs = self.outputs.lock().unwrap();
        let output = outputs.get(&agent_name).ok_or(TaskError::NotFound(id))?;
        Ok(TaskResultStream::Inline(
            serde_json::to_string(output).unwrap(),
        ))
    }

    async fn cancel(&self, _id: TaskId) -> Result<(), TaskError> {
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

fn make_three_step_spec(on_failure_b: OnFailurePolicy) -> PipelineSpec {
    PipelineSpec {
        name: "resume-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 30,
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
                    base_delay_ms: 100,
                },
                timeout_secs: 10,
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
                    base_delay_ms: 100,
                },
                timeout_secs: 10,
                condition: None,
                on_failure: on_failure_b,
            },
            StepSpec {
                id: "C".to_string(),
                agent: "agent-c".to_string(),
                input: json!({}),
                depends_on: vec!["B".to_string()],
                retry: RetrySpec {
                    max_attempts: 1,
                    backoff: BackoffKind::Exponential,
                    base_delay_ms: 100,
                },
                timeout_secs: 10,
                condition: None,
                on_failure: OnFailurePolicy::Fail,
            },
        ],
    }
}

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// TC-PIPE-04: resume after failure at B, step A is NOT re-dispatched.
///
/// 1. First run: A succeeds, B fails (on_failure: Fail). Pipeline state: Failed.
/// 2. Resume: A (already Finished) is skipped, B re-dispatched (succeeds),
///    C dispatched (succeeds). Final state: Finished.
/// 3. Assert: agent-a submitted exactly once, agent-b submitted twice,
///    agent-c submitted once.
#[tokio::test]
async fn resume_after_failure_skips_finished_steps() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut outputs = HashMap::new();
    outputs.insert("agent-a".to_string(), json!({"result": "a-done"}));
    outputs.insert("agent-b".to_string(), json!({"result": "b-done"}));
    outputs.insert("agent-c".to_string(), json!({"result": "c-done"}));

    // agent-b fails on call 1, succeeds on call 2+.
    let mut fail_until = HashMap::new();
    fail_until.insert("agent-b".to_string(), 1u32);

    let mock = Arc::new(ResumeMockTaskService::new(outputs, fail_until));
    let engine = DefaultPipelineEngine::new(store.clone(), mock.clone());

    let spec = make_three_step_spec(OnFailurePolicy::Fail);

    // First run: should fail at B.
    let err = engine
        .run(spec)
        .await
        .expect_err("first run should fail at B");
    assert!(
        matches!(err, PipelineError::StepFailed { ref step, .. } if step == "B"),
        "expected StepFailed at B, got: {err:?}"
    );

    // Get the pipeline ID from the store.
    let runs = store.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let id = runs[0];

    // Verify intermediate state: A=Finished, B=Failed, C=Pending.
    let run = store.load_state(id).expect("load state");
    assert_eq!(run.state, PipelineState::Failed);
    assert_eq!(run.steps["A"].state, StepRunState::Finished);
    assert_eq!(run.steps["B"].state, StepRunState::Failed);
    assert_eq!(run.steps["C"].state, StepRunState::Pending);

    // Resume the pipeline.
    let resumed_id = engine.resume(id).await.expect("resume should succeed");
    assert_eq!(resumed_id, id, "resumed id should match original");

    // Verify final state: Finished.
    let final_run = store.load_state(id).expect("load final state");
    assert_eq!(final_run.state, PipelineState::Finished);
    assert!(final_run.finished_at.is_some());

    // All steps should be Finished.
    for step_id in ["A", "B", "C"] {
        assert_eq!(
            final_run.steps[step_id].state,
            StepRunState::Finished,
            "step {step_id} should be Finished after resume"
        );
    }

    // Call counts: A=1 (not re-dispatched), B=2 (original + resume), C=1.
    assert_eq!(
        mock.call_count("agent-a"),
        1,
        "agent-a should only be submitted once (skipped on resume)"
    );
    assert_eq!(
        mock.call_count("agent-b"),
        2,
        "agent-b should be submitted twice (original fail + resume success)"
    );
    assert_eq!(
        mock.call_count("agent-c"),
        1,
        "agent-c should be submitted once (on resume)"
    );

    // Audit log should contain a Resumed event.
    let audit = read_audit_lines(tmp.path(), id);
    let resumed_count = audit.iter().filter(|e| e["type"] == "resumed").count();
    assert_eq!(
        resumed_count, 1,
        "audit log should contain exactly 1 Resumed event"
    );
}

/// Resume on a finished run is a no-op: returns Ok(same id).
#[tokio::test]
async fn resume_finished_run_is_noop() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    // All agents succeed on first call.
    let mut outputs = HashMap::new();
    outputs.insert("agent-a".to_string(), json!({"result": "a"}));
    outputs.insert("agent-b".to_string(), json!({"result": "b"}));
    outputs.insert("agent-c".to_string(), json!({"result": "c"}));

    let mock = Arc::new(ResumeMockTaskService::new(outputs, HashMap::new()));
    let engine = DefaultPipelineEngine::new(store.clone(), mock.clone());

    let spec = make_three_step_spec(OnFailurePolicy::Fail);

    // First run: all succeed.
    let id = engine.run(spec).await.expect("pipeline should succeed");

    // Verify finished.
    let run = store.load_state(id).expect("load state");
    assert_eq!(run.state, PipelineState::Finished);

    // Resume should be a no-op.
    let resumed_id = engine.resume(id).await.expect("resume should return Ok");
    assert_eq!(resumed_id, id);

    // No additional agent calls should have been made.
    assert_eq!(mock.call_count("agent-a"), 1);
    assert_eq!(mock.call_count("agent-b"), 1);
    assert_eq!(mock.call_count("agent-c"), 1);
}

/// Resume on a rolled-back run returns a ValidationError.
#[tokio::test]
async fn resume_rolled_back_run_returns_error() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mock = Arc::new(ResumeMockTaskService::new(HashMap::new(), HashMap::new()));
    let engine = DefaultPipelineEngine::new(store.clone(), mock);

    // Manually create a run in RolledBack state with a spec on disk.
    let id = archon_pipeline::PipelineId::new();
    let run = archon_pipeline::PipelineRun {
        id,
        spec_hash: "abc123".to_string(),
        state: PipelineState::RolledBack,
        steps: HashMap::new(),
        started_at: Utc::now(),
        finished_at: Some(Utc::now()),
    };
    store.create(&run).expect("create run directory");
    store.save_state(&run).expect("save rolled-back state");

    // Also save a spec (resume loads it).
    let spec = make_three_step_spec(OnFailurePolicy::Fail);
    store.save_spec(id, &spec).expect("save spec");

    // Attempt resume.
    let err = engine
        .resume(id)
        .await
        .expect_err("resume should fail on rolled-back run");
    match err {
        PipelineError::ValidationError(msg) => {
            assert!(
                msg.contains("cannot resume a rolled-back run"),
                "unexpected error message: {msg}"
            );
        }
        other => panic!("expected ValidationError, got: {other:?}"),
    }
}

/// Resume after a fail-policy failure: B has `on_failure: Fail`.
/// First run fails at B. Resume re-dispatches B (succeeds this time),
/// then dispatches C. Final state: Finished.
#[tokio::test]
async fn resume_after_fail_policy_redispatches_failed_step() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut outputs = HashMap::new();
    outputs.insert("agent-a".to_string(), json!({"v": 1}));
    outputs.insert("agent-b".to_string(), json!({"v": 2}));
    outputs.insert("agent-c".to_string(), json!({"v": 3}));

    // agent-b fails on call 1, succeeds on call 2+.
    let mut fail_until = HashMap::new();
    fail_until.insert("agent-b".to_string(), 1u32);

    let mock = Arc::new(ResumeMockTaskService::new(outputs, fail_until));
    let engine = DefaultPipelineEngine::new(store.clone(), mock.clone());

    let spec = make_three_step_spec(OnFailurePolicy::Fail);

    // First run: fails at B.
    let _err = engine.run(spec).await.expect_err("first run should fail");

    let runs = store.list_runs().unwrap();
    let id = runs[0];

    // Pre-resume state.
    let pre_run = store.load_state(id).unwrap();
    assert_eq!(pre_run.state, PipelineState::Failed);
    assert_eq!(pre_run.steps["B"].state, StepRunState::Failed);
    assert!(pre_run.steps["B"].last_error.is_some());

    // Resume.
    let resumed_id = engine.resume(id).await.expect("resume should succeed");
    assert_eq!(resumed_id, id);

    // Final state.
    let final_run = store.load_state(id).unwrap();
    assert_eq!(final_run.state, PipelineState::Finished);

    // B should now be Finished with output and reset attempts.
    assert_eq!(final_run.steps["B"].state, StepRunState::Finished);
    assert!(final_run.steps["B"].output.is_some());
    // The resume resets attempts to 0, then executor increments to 1 on dispatch.
    assert_eq!(final_run.steps["B"].attempts, 1);
    // Last error should be cleared (step succeeded on resume).
    assert!(final_run.steps["B"].last_error.is_none());

    // C should have been dispatched and finished.
    assert_eq!(final_run.steps["C"].state, StepRunState::Finished);
    assert!(final_run.steps["C"].output.is_some());

    // Verify call counts.
    assert_eq!(mock.call_count("agent-a"), 1, "A not re-dispatched");
    assert_eq!(mock.call_count("agent-b"), 2, "B dispatched twice");
    assert_eq!(mock.call_count("agent-c"), 1, "C dispatched once on resume");
}
