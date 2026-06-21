use archon_workflow::{
    HeuristicWorkflowPlanner, RunStatus, StageRunOutput, StageRunRequest, StageStatus,
    TemplateRegistry, WorkflowExecutor, WorkflowPlanner, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore, stage::source_input_hash,
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
async fn failed_stage_with_downstream_remediation_continues() {
    struct RecoverableFailureRunner;

    impl archon_workflow::WriteBoundaryProbe for RecoverableFailureRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for RecoverableFailureRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "remediation-inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), write_permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: recoverable-failure
task: Continue into remediation when a write stage fails.
stages:
  - id: implement
    kind: agent
    provider_tier: local
    verify_command: "false"
  - id: final_artifacts
    kind: agent
    depends_on: [implement]
  - id: remediation-inventory
    kind: agent
    outputs: [items]
    depends_on: [final_artifacts, implement]
  - id: remediate
    kind: fanout
    foreach: "${remediation-inventory.items}"
    item_kind: implementation
    allow_empty_items: true
    depends_on: [remediation-inventory]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &RecoverableFailureRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished.stages.get("remediation-inventory").unwrap().status,
        StageStatus::Accepted
    );
    assert_eq!(
        finished.stages.get("remediate").unwrap().status,
        StageStatus::Failed
    );
    assert!(
        finished
            .stages
            .get("remediate")
            .and_then(|stage| stage.error.as_deref())
            .unwrap_or_default()
            .contains("unresolved forced-accepted failure")
    );
}

#[tokio::test]
async fn remediation_context_includes_failed_fanout_item_artifacts() {
    struct FailedFanoutItemRunner;

    impl archon_workflow::WriteBoundaryProbe for FailedFanoutItemRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for FailedFanoutItemRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "fan-0" => Ok(StageRunOutput::markdown(r#"{"result":"failed"}"#)),
                "remediation-inventory" => {
                    let artifacts = request.input["dependencies"][0]["artifacts"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    assert!(
                        artifacts.iter().any(|artifact| artifact["content"]
                            .as_str()
                            .is_some_and(|content| content.contains(r#""result":"failed""#))),
                        "failed fanout item artifact was not visible: {artifacts:?}"
                    );
                    Ok(StageRunOutput::markdown(r#"{"items":[]}"#))
                }
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), write_permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: failed-item-context
task: Preserve failed fanout item evidence for remediation.
stages:
  - id: fan
    kind: fanout
    input:
      items:
        - name: one
  - id: remediation-inventory
    kind: agent
    outputs: [items]
    depends_on: [fan]
  - id: remediate
    kind: fanout
    foreach: "${remediation-inventory.items}"
    item_kind: implementation
    allow_empty_items: true
    depends_on: [remediation-inventory]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &FailedFanoutItemRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(
        finished.stages.get("fan").unwrap().status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished.stages.get("remediation-inventory").unwrap().status,
        StageStatus::Accepted
    );
    assert_eq!(
        finished.stages.get("remediate").unwrap().status,
        StageStatus::Failed
    );
    assert!(
        finished
            .stages
            .get("remediate")
            .and_then(|stage| stage.error.as_deref())
            .unwrap_or_default()
            .contains("unresolved forced-accepted failure")
    );
}

#[tokio::test]
async fn remediation_item_absolute_project_targets_use_project_root() {
    struct ProjectRootRunner {
        expected_root: String,
        target: String,
    }

    impl archon_workflow::WriteBoundaryProbe for ProjectRootRunner {
        fn supports_workspace_boundary(&self) -> bool {
            true
        }
    }
    #[async_trait::async_trait]
    impl WorkflowStageRunner for ProjectRootRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "inventory" => Ok(StageRunOutput::markdown(format!(
                    r#"{{"items":[{{"target_files":["{}"],"task":"repair report"}}]}}"#,
                    self.target
                ))),
                "repair-0" => {
                    assert_eq!(
                        request
                            .input
                            .get("target_repository_root")
                            .and_then(serde_json::Value::as_str),
                        Some(self.expected_root.as_str())
                    );
                    assert!(
                        request.input.get("write_coordination").is_none(),
                        "external project target must use serial fanout path"
                    );
                    std::fs::create_dir_all(std::path::Path::new(&self.target).parent().unwrap())
                        .unwrap();
                    std::fs::write(&self.target, "fixed").unwrap();
                    Ok(StageRunOutput::markdown("status: completed"))
                }
                _ => Ok(StageRunOutput::markdown("status: verified")),
            }
        }
    }

    let project = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join(".git"), "gitdir: elsewhere").unwrap();
    let target = project.path().join("tasks/report.md");
    let store = WorkflowStore::project(project.path());
    let executor = WorkflowExecutor::new(store.clone(), write_permissive_policy());
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: project-target-remediation
task: Repair project task artifacts.
target_repository_root: "{repo}"
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: repair
    kind: fanout
    foreach: "${{inventory.items}}"
    item_kind: implementation
    depends_on: [inventory]
"#,
        repo = repo.path().display()
    ))
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(
            run.clone(),
            &ProjectRootRunner {
                expected_root: project.path().display().to_string(),
                target: target.display().to_string(),
            },
        )
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(std::fs::read_to_string(target).unwrap(), "fixed");
}

#[tokio::test]
async fn implementation_target_inventory_empty_items_fails_fast() {
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
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Failed
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
                tool_uses: Vec::new(),
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
