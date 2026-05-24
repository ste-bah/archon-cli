use super::*;

use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use futures_util::Stream;
use serde_json::json;
use tempfile::TempDir;

use archon_core::tasks::{
    SubmitRequest, TaskError, TaskEvent, TaskFilter, TaskId, TaskResultStream, TaskService,
    TaskSnapshot, TaskState,
};

use crate::spec::{BackoffKind, OnFailurePolicy, PipelineSpec, RetrySpec, StepSpec};

// -----------------------------------------------------------------------
// Mock TaskService
// -----------------------------------------------------------------------

/// Recorded call from submit().
#[derive(Debug, Clone)]
struct SubmitRecord {
    agent_name: String,
    input: serde_json::Value,
    task_id: TaskId,
}

/// Configurable mock task service for pipeline executor tests.
struct MockTaskService {
    /// Maps agent_name to (output_value, should_fail).
    responses: Mutex<HashMap<String, (serde_json::Value, bool)>>,
    /// Tracks all submit calls in order.
    submissions: Mutex<Vec<SubmitRecord>>,
    call_count: AtomicU32,
}

impl MockTaskService {
    fn new(responses: HashMap<String, (serde_json::Value, bool)>) -> Self {
        Self {
            responses: Mutex::new(responses),
            submissions: Mutex::new(Vec::new()),
            call_count: AtomicU32::new(0),
        }
    }

    /// Return all recorded submissions.
    fn get_submissions(&self) -> Vec<SubmitRecord> {
        self.submissions.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaskService for MockTaskService {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let task_id = TaskId::new();
        self.submissions.lock().unwrap().push(SubmitRecord {
            agent_name: req.agent_name,
            input: req.input,
            task_id,
        });
        Ok(task_id)
    }

    async fn status(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        // Find the submission by task_id to get the agent_name.
        let submissions = self.submissions.lock().unwrap();
        let record = submissions
            .iter()
            .find(|r| r.task_id == id)
            .ok_or(TaskError::NotFound(id))?;

        let responses = self.responses.lock().unwrap();
        let (_output, should_fail) = responses
            .get(&record.agent_name)
            .ok_or(TaskError::NotFound(id))?;

        let state = if *should_fail {
            TaskState::Failed
        } else {
            TaskState::Finished
        };

        Ok(TaskSnapshot {
            id,
            agent_name: record.agent_name.clone(),
            state,
            progress_pct: None,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
            error: if *should_fail {
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
        let (output, should_fail) = responses
            .get(&record.agent_name)
            .ok_or(TaskError::NotFound(id))?;

        if *should_fail {
            return Err(TaskError::NotFound(id));
        }

        Ok(TaskResultStream::Inline(
            serde_json::to_string(output).unwrap(),
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

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn make_spec(steps: Vec<(&str, &str, Vec<&str>, serde_json::Value)>) -> PipelineSpec {
    PipelineSpec {
        name: "test-pipeline".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 3600,
        max_parallelism: 1,
        steps: steps
            .into_iter()
            .map(|(id, agent, deps, input)| StepSpec {
                id: id.to_string(),
                agent: agent.to_string(),
                input,
                depends_on: deps.into_iter().map(|d| d.to_string()).collect(),
                retry: RetrySpec {
                    max_attempts: 1,
                    backoff: BackoffKind::Exponential,
                    base_delay_ms: 1000,
                },
                timeout_secs: 1800,
                condition: None,
                on_failure: OnFailurePolicy::Fail,
            })
            .collect(),
    }
}

/// Read the audit log lines from disk for a given pipeline run.
fn read_audit_lines(store_root: &std::path::Path, id: PipelineId) -> Vec<serde_json::Value> {
    let audit_path = store_root.join(id.to_string()).join("audit.log");
    let raw = std::fs::read_to_string(&audit_path).unwrap_or_default();
    raw.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("valid audit JSON"))
        .collect()
}

/// Check whether a checkpoint file exists for a given step.
fn checkpoint_exists(store_root: &std::path::Path, id: PipelineId, step_id: &str) -> bool {
    store_root
        .join(id.to_string())
        .join("checkpoints")
        .join(format!("{step_id}.json"))
        .exists()
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[tokio::test]
async fn three_step_linear_success() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    // A -> B -> C, each outputs {"count": N}
    let mut responses = HashMap::new();
    responses.insert("agent-a".to_string(), (json!({"count": 1}), false));
    responses.insert("agent-b".to_string(), (json!({"count": 2}), false));
    responses.insert("agent-c".to_string(), (json!({"count": 3}), false));

    let mock = Arc::new(MockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_spec(vec![
        ("A", "agent-a", vec![], json!({})),
        ("B", "agent-b", vec!["A"], json!({"prev": "${A.output}"})),
        ("C", "agent-c", vec!["B"], json!({"prev": "${B.output}"})),
    ]);

    let id = executor.run(spec).await.expect("pipeline should succeed");

    // Verify pipeline state.
    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);
    assert!(run.finished_at.is_some());

    // All 3 steps should be Finished.
    for step_id in ["A", "B", "C"] {
        let step_run = run.steps.get(step_id).expect("step should exist");
        assert_eq!(step_run.state, StepRunState::Finished, "step {step_id}");
        assert!(
            step_run.output.is_some(),
            "step {step_id} should have output"
        );
    }

    // 3 checkpoints should exist.
    for step_id in ["A", "B", "C"] {
        assert!(
            checkpoint_exists(tmp.path(), id, step_id),
            "checkpoint for {step_id} should exist"
        );
    }

    // Audit log: Started + 3x(StepStarted + StepFinished) + Finished = 8 lines.
    let audit = read_audit_lines(tmp.path(), id);
    assert_eq!(
        audit.len(),
        8,
        "expected 8 audit lines, got {}",
        audit.len()
    );

    // First event is Started, last is Finished.
    assert_eq!(audit[0]["type"], "started");
    assert_eq!(audit[audit.len() - 1]["type"], "finished");

    // 3 submit calls.
    assert_eq!(mock.call_count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn failure_mid_pipeline() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    // A succeeds, B fails, C should never run.
    let mut responses = HashMap::new();
    responses.insert("agent-a".to_string(), (json!({"ok": true}), false));
    responses.insert("agent-b".to_string(), (json!(null), true)); // fails
    responses.insert("agent-c".to_string(), (json!({"ok": true}), false));

    let mock = Arc::new(MockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_spec(vec![
        ("A", "agent-a", vec![], json!({})),
        ("B", "agent-b", vec!["A"], json!({})),
        ("C", "agent-c", vec!["B"], json!({})),
    ]);

    let err = executor.run(spec).await.expect_err("pipeline should fail");
    match err {
        PipelineError::StepFailed { step, .. } => {
            assert_eq!(step, "B", "step B should be the failure");
        }
        other => panic!("expected StepFailed, got: {other:?}"),
    }

    // Load the persisted state — the executor saves before returning the error.
    let runs = store.list_runs().expect("list runs");
    assert_eq!(runs.len(), 1);
    let id = runs[0];

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Failed);

    // A should be Finished, B should be Failed, C should be Pending.
    assert_eq!(run.steps["A"].state, StepRunState::Finished);
    assert_eq!(run.steps["B"].state, StepRunState::Failed);
    assert_eq!(run.steps["C"].state, StepRunState::Pending);

    // Checkpoint for A exists, not for B or C.
    assert!(checkpoint_exists(tmp.path(), id, "A"));
    assert!(!checkpoint_exists(tmp.path(), id, "B"));
    assert!(!checkpoint_exists(tmp.path(), id, "C"));

    // Audit should contain StepFailed for B.
    let audit = read_audit_lines(tmp.path(), id);
    let has_step_failed = audit
        .iter()
        .any(|e| e["type"] == "step_failed" && e["step"] == "B");
    assert!(has_step_failed, "audit should contain step_failed for B");

    // Only 2 submit calls (A and B), C was never submitted.
    assert_eq!(mock.call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn variable_substitution_carries_through() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    // A outputs {"data": 42}, B's input references ${A.output.data}.
    let mut responses = HashMap::new();
    responses.insert("agent-a".to_string(), (json!({"data": 42}), false));
    responses.insert("agent-b".to_string(), (json!({"result": "ok"}), false));

    let mock = Arc::new(MockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_spec(vec![
        ("A", "agent-a", vec![], json!({})),
        (
            "B",
            "agent-b",
            vec!["A"],
            json!({"ref": "${A.output.data}"}),
        ),
    ]);

    let _id = executor.run(spec).await.expect("pipeline should succeed");

    // Verify the mock received the resolved input for B.
    let submissions = mock.get_submissions();
    assert_eq!(submissions.len(), 2);

    let b_submission = submissions
        .iter()
        .find(|s| s.agent_name == "agent-b")
        .expect("agent-b should have been submitted");

    // The key check: ${A.output.data} should resolve to the number 42,
    // preserving type (not the string "42").
    assert_eq!(
        b_submission.input,
        json!({"ref": 42}),
        "variable substitution should resolve ${{A.output.data}} to the number 42"
    );
}
