//! Integration tests for the pipeline engine API as used by the CLI.
//!
//! Tests parse -> validate -> run/status/cancel/resume/list flow
//! using the DefaultPipelineEngine with mock TaskService.

use std::pin::Pin;
use std::sync::Arc;

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
    PipelineFormat, PipelineSpec, PipelineState, PipelineStateStore, RetrySpec, StepSpec,
};

// -- Simple mock that always succeeds --

struct AlwaysSucceedMock {
    submissions: std::sync::Mutex<Vec<(String, TaskId)>>,
}

impl AlwaysSucceedMock {
    fn new() -> Self {
        Self {
            submissions: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl TaskService for AlwaysSucceedMock {
    async fn submit(&self, req: SubmitRequest) -> Result<TaskId, TaskError> {
        let id = TaskId::new();
        self.submissions.lock().unwrap().push((req.agent_name, id));
        Ok(id)
    }

    async fn status(&self, id: TaskId) -> Result<TaskSnapshot, TaskError> {
        let subs = self.submissions.lock().unwrap();
        let (agent, _) = subs.iter().find(|(_, tid)| *tid == id).ok_or(TaskError::NotFound(id))?;
        Ok(TaskSnapshot {
            id,
            agent_name: agent.clone(),
            state: TaskState::Finished,
            progress_pct: None,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
            error: None,
        })
    }

    async fn result(&self, _id: TaskId, _stream: bool) -> Result<TaskResultStream, TaskError> {
        Ok(TaskResultStream::Inline(json!({"ok": true}).to_string()))
    }

    async fn cancel(&self, _id: TaskId) -> Result<(), TaskError> {
        Ok(())
    }

    async fn subscribe_events(
        &self, _id: TaskId, _from_seq: u64,
    ) -> Result<Pin<Box<dyn Stream<Item = TaskEvent> + Send>>, TaskError> {
        Err(TaskError::Unimplemented)
    }

    async fn list(&self, _filter: TaskFilter) -> Result<Vec<TaskSnapshot>, TaskError> {
        Ok(vec![])
    }
}

// -- Tests --

/// Parse a YAML fixture and validate it produces a valid DAG.
#[tokio::test]
async fn parse_and_validate_linear_yaml() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));
    let mock = Arc::new(AlwaysSucceedMock::new());
    let engine = DefaultPipelineEngine::new(store, mock);

    let yaml = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/linear.yaml"),
    ).unwrap();

    let spec = engine.parse(&yaml, PipelineFormat::Yaml).unwrap();
    assert_eq!(spec.name, "linear-test");
    assert_eq!(spec.steps.len(), 3);

    let dag = engine.validate(&spec).unwrap();
    assert_eq!(dag.levels.len(), 3); // A -> B -> C
}

/// Cyclic pipeline fixture fails validation with CycleDetected.
#[tokio::test]
async fn cyclic_yaml_fails_validation() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));
    let mock = Arc::new(AlwaysSucceedMock::new());
    let engine = DefaultPipelineEngine::new(store, mock);

    let yaml = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cyclic.yaml"),
    ).unwrap();

    let spec = engine.parse(&yaml, PipelineFormat::Yaml).unwrap();
    let err = engine.validate(&spec).expect_err("cyclic should fail");
    match err {
        PipelineError::CycleDetected(ids) => {
            assert!(ids.contains(&"A".to_string()));
            assert!(ids.contains(&"B".to_string()));
        }
        other => panic!("expected CycleDetected, got: {other:?}"),
    }
}

/// Missing ref fixture fails validation with MissingStep.
#[tokio::test]
async fn missing_ref_yaml_fails_validation() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));
    let mock = Arc::new(AlwaysSucceedMock::new());
    let engine = DefaultPipelineEngine::new(store, mock);

    let yaml = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/missing_ref.yaml"),
    ).unwrap();

    let spec = engine.parse(&yaml, PipelineFormat::Yaml).unwrap();
    let err = engine.validate(&spec).expect_err("missing ref should fail");
    match err {
        PipelineError::MissingStep(id) => assert_eq!(id, "ghost"),
        other => panic!("expected MissingStep, got: {other:?}"),
    }
}

/// Full lifecycle: run -> status -> list.
#[tokio::test]
async fn run_status_list_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));
    let mock = Arc::new(AlwaysSucceedMock::new());
    let engine = DefaultPipelineEngine::new(store.clone(), mock);

    let spec = PipelineSpec {
        name: "lifecycle-test".to_string(),
        version: "1.0".to_string(),
        global_timeout_secs: 30,
        max_parallelism: 1,
        steps: vec![
            StepSpec {
                id: "A".to_string(),
                agent: "agent-a".to_string(),
                input: json!({}),
                depends_on: vec![],
                retry: RetrySpec { max_attempts: 1, backoff: BackoffKind::Exponential, base_delay_ms: 100 },
                timeout_secs: 10,
                condition: None,
                on_failure: OnFailurePolicy::Fail,
            },
        ],
    };

    // Run
    let id = engine.run(spec).await.expect("run should succeed");

    // Status
    let run = engine.status(id).await.expect("status should succeed");
    assert_eq!(run.state, PipelineState::Finished);
    assert_eq!(run.steps["A"].state, archon_pipeline::StepRunState::Finished);

    // List
    let ids = engine.list().await.expect("list should succeed");
    assert!(ids.contains(&id), "list should contain the run id");

    // Verify spec.json exists on disk.
    let spec_path = tmp.path().join(id.to_string()).join("spec.json");
    assert!(spec_path.exists(), "spec.json should exist after run");
}

/// Parse JSON format works too.
#[tokio::test]
async fn parse_json_format() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));
    let mock = Arc::new(AlwaysSucceedMock::new());
    let engine = DefaultPipelineEngine::new(store, mock);

    let json_src = r#"{"name":"json-test","steps":[{"id":"X","agent":"a"}]}"#;
    let spec = engine.parse(json_src, PipelineFormat::Json).unwrap();
    assert_eq!(spec.name, "json-test");
    assert_eq!(spec.steps.len(), 1);
}

/// Box<dyn PipelineEngine> is constructable (trait object safety).
#[tokio::test]
async fn trait_object_constructable() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(PipelineStateStore::new(tmp.path()));
    let mock: Arc<dyn TaskService> = Arc::new(AlwaysSucceedMock::new());

    let engine: Box<dyn PipelineEngine> = Box::new(
        DefaultPipelineEngine::new(store, mock),
    );

    let json_src = r#"{"name":"obj-test","steps":[{"id":"X","agent":"a"}]}"#;
    let spec = engine.parse(json_src, PipelineFormat::Json).unwrap();
    assert_eq!(spec.name, "obj-test");
}
