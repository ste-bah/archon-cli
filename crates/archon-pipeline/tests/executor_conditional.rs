//! Integration tests for CEL-based conditional step execution.
//!
//! Tests that the `condition` field on a [`StepSpec`] is evaluated before the
//! step's retry loop runs, skipping the step when the expression yields `false`.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

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
// Mock TaskService
// ---------------------------------------------------------------------------

/// Recorded call from submit().
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SubmitRecord {
    agent_name: String,
    input: serde_json::Value,
    task_id: TaskId,
}

/// Mock task service with per-agent responses.
struct ConditionalMockTaskService {
    responses: std::sync::Mutex<HashMap<String, serde_json::Value>>,
    submissions: std::sync::Mutex<Vec<SubmitRecord>>,
    call_count: AtomicU32,
}

impl ConditionalMockTaskService {
    fn new(responses: HashMap<String, serde_json::Value>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses),
            submissions: std::sync::Mutex::new(Vec::new()),
            call_count: AtomicU32::new(0),
        }
    }

    fn get_submissions(&self) -> Vec<SubmitRecord> {
        self.submissions.lock().unwrap().clone()
    }
}

#[async_trait]
impl TaskService for ConditionalMockTaskService {
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
        let submissions = self.submissions.lock().unwrap();
        let record = submissions
            .iter()
            .find(|r| r.task_id == id)
            .ok_or(TaskError::NotFound(id))?;

        Ok(TaskSnapshot {
            id,
            agent_name: record.agent_name.clone(),
            state: TaskState::Finished,
            progress_pct: None,
            created_at: chrono::Utc::now(),
            started_at: Some(chrono::Utc::now()),
            finished_at: Some(chrono::Utc::now()),
            error: None,
        })
    }

    async fn result(&self, id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
        let submissions = self.submissions.lock().unwrap();
        let record = submissions
            .iter()
            .find(|r| r.task_id == id)
            .ok_or(TaskError::NotFound(id))?;

        let responses = self.responses.lock().unwrap();
        let output = responses
            .get(&record.agent_name)
            .ok_or(TaskError::NotFound(id))?;

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_step(
    id: &str,
    agent: &str,
    deps: Vec<&str>,
    input: serde_json::Value,
    condition: Option<&str>,
) -> StepSpec {
    StepSpec {
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
        condition: condition.map(|s| s.to_string()),
        on_failure: OnFailurePolicy::Fail,
    }
}

fn make_pipeline(steps: Vec<StepSpec>) -> PipelineSpec {
    PipelineSpec {
        name: "conditional-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 3600,
        max_parallelism: 1,
        steps,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A -> B -> C, where A outputs `{"should_run": true}` and B has
/// `condition: "${a.output.should_run}"`.  Since the condition is true,
/// all three steps should run and finish.
#[tokio::test]
async fn condition_true_step_runs() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert("agent-a".to_string(), json!({"should_run": true}));
    responses.insert("agent-b".to_string(), json!({"result": "b-ok"}));
    responses.insert("agent-c".to_string(), json!({"result": "c-ok"}));

    let mock = Arc::new(ConditionalMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    let spec = make_pipeline(vec![
        make_step("a", "agent-a", vec![], json!({}), None),
        make_step(
            "b",
            "agent-b",
            vec!["a"],
            json!({"prev": "${a.output}"}),
            Some("${a.output.should_run}"),
        ),
        make_step(
            "c",
            "agent-c",
            vec!["b"],
            json!({"prev": "${b.output}"}),
            None,
        ),
    ]);

    let id = executor.run(spec).await.expect("pipeline should succeed");

    // Verify pipeline state.
    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);

    // All three steps should be Finished.
    for step_id in ["a", "b", "c"] {
        let step_run = run.steps.get(step_id).expect("step should exist");
        assert_eq!(
            step_run.state,
            StepRunState::Finished,
            "step {step_id} should be Finished"
        );
        assert!(
            step_run.output.is_some(),
            "step {step_id} should have output"
        );
    }

    // All three checkpoints should exist.
    for step_id in ["a", "b", "c"] {
        assert!(
            checkpoint_exists(tmp.path(), id, step_id),
            "checkpoint for {step_id} should exist"
        );
    }

    // 3 submit calls (all steps ran).
    assert_eq!(mock.call_count.load(Ordering::SeqCst), 3);

    // No StepSkipped events in audit.
    let audit = read_audit_lines(tmp.path(), id);
    let skip_count = audit
        .iter()
        .filter(|e| e["type"] == "step_skipped")
        .count();
    assert_eq!(skip_count, 0, "no steps should be skipped");
}

/// A -> B, C (independent). A outputs `{"should_run": false}`, B has
/// `condition: "${a.output.should_run}"` and depends on A.  C is independent.
/// Expected: A runs, B is skipped (condition false), C runs.
/// Audit has StepSkipped for B.  B has no checkpoint.
#[tokio::test]
async fn condition_false_step_skipped() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));

    let mut responses = HashMap::new();
    responses.insert("agent-a".to_string(), json!({"should_run": false}));
    responses.insert("agent-b".to_string(), json!({"result": "b-ok"}));
    responses.insert("agent-c".to_string(), json!({"result": "c-ok"}));

    let mock = Arc::new(ConditionalMockTaskService::new(responses));
    let executor = PipelineExecutor::new(store.clone(), mock.clone());

    // Pipeline:
    //   Level 0: a (no deps)
    //   Level 1: b (depends on a, has condition), c (depends on a, no condition)
    let spec = make_pipeline(vec![
        make_step("a", "agent-a", vec![], json!({}), None),
        make_step(
            "b",
            "agent-b",
            vec!["a"],
            json!({}),
            Some("${a.output.should_run}"),
        ),
        make_step("c", "agent-c", vec!["a"], json!({}), None),
    ]);

    let id = executor.run(spec).await.expect("pipeline should succeed");

    let run = store.load_state(id).expect("state should load");
    assert_eq!(run.state, PipelineState::Finished);

    // A should be Finished.
    assert_eq!(run.steps["a"].state, StepRunState::Finished);
    assert!(run.steps["a"].output.is_some());

    // B should be Skipped (condition was false).
    assert_eq!(run.steps["b"].state, StepRunState::Skipped);
    assert!(
        run.steps["b"].output.is_none(),
        "skipped step should have no output"
    );
    assert!(
        run.steps["b"].task_id.is_none(),
        "skipped step should have no task_id"
    );

    // C should be Finished.
    assert_eq!(run.steps["c"].state, StepRunState::Finished);
    assert!(run.steps["c"].output.is_some());

    // Checkpoint for A and C exists, NOT for B.
    assert!(checkpoint_exists(tmp.path(), id, "a"));
    assert!(
        !checkpoint_exists(tmp.path(), id, "b"),
        "skipped step should have no checkpoint"
    );
    assert!(checkpoint_exists(tmp.path(), id, "c"));

    // Only 2 submit calls: A and C.  B was never submitted.
    let submissions = mock.get_submissions();
    let agent_names: Vec<&str> = submissions.iter().map(|s| s.agent_name.as_str()).collect();
    assert_eq!(submissions.len(), 2);
    assert!(agent_names.contains(&"agent-a"));
    assert!(agent_names.contains(&"agent-c"));
    assert!(
        !agent_names.contains(&"agent-b"),
        "agent-b should never be submitted"
    );

    // Audit log should contain StepSkipped for B.
    let audit = read_audit_lines(tmp.path(), id);
    let skip_events: Vec<&serde_json::Value> = audit
        .iter()
        .filter(|e| e["type"] == "step_skipped")
        .collect();
    assert_eq!(skip_events.len(), 1, "expected 1 StepSkipped event");
    assert_eq!(skip_events[0]["step"], "b");
}
