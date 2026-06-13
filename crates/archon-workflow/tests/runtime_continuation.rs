use archon_workflow::{
    HeuristicWorkflowPlanner, StageRunOutput, StageRunRequest, StageStatus, TemplateRegistry,
    WorkflowExecutor, WorkflowPlanner, WorkflowPolicy, WorkflowSpec, WorkflowStageRunner,
    WorkflowStore, stage::source_input_hash,
};

fn write_permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn remediation_fanout_allows_empty_items_when_explicit() {
    struct EmptyInventoryRunner;

    impl archon_workflow::WriteBoundaryProbe for EmptyInventoryRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyInventoryRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "inventory" {
                return Ok(StageRunOutput::markdown(r#"{"items":[]}"#));
            }
            Ok(StageRunOutput::markdown("unexpected remediation item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), write_permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-remediation
task: No-op clean remediation.
stages:
  - id: inventory
    kind: agent
    agent: critic
    outputs: [items]
  - id: remediate
    kind: fanout
    foreach: "${inventory.items}"
    item_kind: implementation
    allow_empty_items: true
    depends_on: [inventory]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyInventoryRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("remediate").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn generated_target_inventory_empty_items_noops_legacy_specs() {
    struct EmptyTargetInventoryRunner;

    impl archon_workflow::WriteBoundaryProbe for EmptyTargetInventoryRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyTargetInventoryRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "implement-target-inventory" {
                return Ok(StageRunOutput::markdown(r#"{"items":[]}"#));
            }
            Ok(StageRunOutput::markdown("unexpected implementation item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), write_permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-generated-target-inventory
task: No-op generated implementation.
stages:
  - id: implement-target-inventory
    kind: agent
    agent: coder
    outputs: [items]
  - id: implement
    kind: fanout
    foreach: "${implement-target-inventory.items}"
    item_kind: implementation
    depends_on: [implement-target-inventory]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyTargetInventoryRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn provider_matrix_executes_code_and_research_workflows() {
    struct MatrixRunner {
        provider: &'static str,
    }

    impl archon_workflow::WriteBoundaryProbe for MatrixRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for MatrixRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            let body = if request.stage_id == "discover" {
                format!(
                    r#"{{"items":[{{"provider":"{}","unit":"a"}},{{"provider":"{}","unit":"b"}}]}}"#,
                    self.provider, self.provider
                )
            } else {
                format!("{} handled {}", self.provider, request.stage_id)
            };
            Ok(StageRunOutput {
                body,
                extension: "md".into(),
                provider_id: Some(self.provider.into()),
                resolved_model: Some(format!("{}-test-model", self.provider)),
                tokens_in: 1,
                tokens_out: 1,
                cost_usd: 0.0,
            })
        }
    }

    for provider in [
        "anthropic",
        "openai-codex",
        "gemini",
        "deepseek",
        "ollama",
        "lm-studio",
    ] {
        for task in [
            "Audit this repo with subagents",
            "Research dynamic workflows",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let store = WorkflowStore::new(temp.path().join("workflows"));
            let spec = HeuristicWorkflowPlanner.plan(task).unwrap();
            let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
            let run = executor.start(spec).unwrap();
            let report = executor
                .execute_with_runner(run, &MatrixRunner { provider })
                .await
                .unwrap();
            assert_eq!(report.failed, 0, "{provider} failed {task}");
        }
    }
}

#[test]
fn template_save_rejects_embedded_secret_text() {
    let temp = tempfile::tempdir().unwrap();
    let mut spec = HeuristicWorkflowPlanner.plan("Research a topic").unwrap();
    spec.stages[0].input = serde_json::json!({
        "note": "Authorization: Bearer should-not-be-saved"
    });
    let err = TemplateRegistry::new(temp.path().join("templates"))
        .save("unsafe", &spec)
        .unwrap_err();
    assert!(err.to_string().contains("credential-like"));
}

#[test]
fn crash_after_artifact_write_resumes_without_duplicate_acceptance() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner
        .plan("Audit this repo with subagents")
        .unwrap();
    let discover = spec
        .stages
        .iter()
        .find(|stage| stage.id == "discover")
        .unwrap()
        .clone();
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    store
        .write_artifact(
            &run.id,
            "discover",
            &source_input_hash(&discover),
            "md",
            b"artifact written before crash",
        )
        .unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    let discover_state = finished.stages.get("discover").unwrap();
    assert_eq!(discover_state.status, StageStatus::Accepted);
    assert_eq!(discover_state.artifacts.len(), 1);
}
