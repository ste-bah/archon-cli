use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};
use std::sync::Mutex;

#[tokio::test]
async fn foreach_items_parse_from_full_dependency_artifact() {
    struct LargeInventoryRunner;

    impl archon_workflow::WriteBoundaryProbe for LargeInventoryRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for LargeInventoryRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                let filler = "x".repeat(40_000);
                let root = request.input["target_repository_root"].as_str().unwrap();
                return Ok(StageRunOutput::markdown(format!(
                    r#"{{"notes":"{filler}","items":[{{"task_ids":["T001"],"target_repository_root":"{root}","target_files":["src/lib.rs"],"task":"edit one file"}}]}}"#
                )));
            }
            let root = request.input["target_repository_root"].as_str().unwrap();
            let target = std::path::Path::new(root).join("src/lib.rs");
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, "// implemented\n").unwrap();
            Ok(implementation_evidence(
                "T001",
                &["src/lib.rs"],
                "fixture verified implement",
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: large-items
task: Exercise large fanout inventory parsing.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${discover.items}
    depends_on: [discover]
"#,
    )
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &LargeInventoryRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(finished.status, RunStatus::Completed);
    assert!(finished.items.contains_key("implement-0"));
}

#[tokio::test]
async fn foreach_filter_runs_only_matching_items() {
    struct FilteredRunner {
        seen: Mutex<Vec<String>>,
    }

    impl archon_workflow::WriteBoundaryProbe for FilteredRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for FilteredRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                let root = request.input["target_repository_root"].as_str().unwrap();
                return Ok(StageRunOutput::markdown(format!(
                    r#"{{
                        "items": [
                            {{"wave_id":"wave1","task_ids":["T001"],"target_repository_root":"{root}","target_files":["src/lib.rs"],"task":"edit T001"}},
                            {{"wave_id":"wave5","task_ids":["T005"],"target_repository_root":"{root}","target_files":["src/ahdm.rs"],"task":"edit AHDM"}}
                        ]
                    }}"#
                )));
            }

            let wave = request.input["fanout_item"]["wave_id"]
                .as_str()
                .unwrap()
                .to_string();
            self.seen
                .lock()
                .unwrap()
                .push(format!("{}:{wave}", request.stage_id));
            let root = request.input["target_repository_root"].as_str().unwrap();
            let target = std::path::Path::new(root).join(
                request.input["fanout_item"]["target_files"][0]
                    .as_str()
                    .unwrap(),
            );
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, format!("// implemented {wave}\n")).unwrap();
            Ok(implementation_evidence(
                "T001",
                &["src/lib.rs"],
                "fixture verified filtered implement",
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let store = WorkflowStore::project(temp.path().join("project"));
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: filtered-items
task: Exercise filtered fanout inventory parsing.
target_repository_root: {}
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${{discover.items}}
    filter: item.wave_id == 'wave1'
    depends_on: [discover]
"#,
        repo.display()
    ))
    .unwrap();
    let runner = FilteredRunner {
        seen: Mutex::new(Vec::new()),
    };

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        runner.seen.lock().unwrap().as_slice(),
        ["implement-0:wave1"]
    );
    assert!(!repo.join("src/ahdm.rs").exists());
    assert!(finished.items.contains_key("implement-0"));
    assert!(!finished.items.contains_key("implement-1"));
}

fn implementation_evidence(task_id: &str, changed_files: &[&str], command: &str) -> StageRunOutput {
    StageRunOutput::markdown(
        serde_json::json!({
            "status": "implemented",
            "implemented_task_ids": [task_id],
            "changed_files": changed_files,
            "commands_run": [{
                "role": "verification",
                "command": command,
                "exit_status": 0
            }],
            "residual_gaps": []
        })
        .to_string(),
    )
}

#[tokio::test]
async fn empty_filtered_implementation_requires_completed_items_proof() {
    struct MissingProofRunner;

    impl archon_workflow::WriteBoundaryProbe for MissingProofRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for MissingProofRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{"items":[{"wave":"wave2","target_files":["src/core.rs"],"task":"edit T010"}]}"#,
                ));
            }
            Ok(StageRunOutput::markdown("unexpected item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-filter-needs-proof
task: Do not infer completion from missing inventory items.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${discover.items}
    filter: item.wave == 'wave1'
    allow_empty_when_completed: true
    completion_task_ids: [T001]
    depends_on: [discover]
"#,
    )
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &MissingProofRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Failed);
    assert!(
        stage
            .error
            .as_deref()
            .unwrap()
            .contains("completed_items proof")
    );
}

#[tokio::test]
async fn empty_filtered_implementation_accepts_matching_completion_proof() {
    struct CompletionProofRunner {
        seen: Mutex<Vec<String>>,
    }

    impl archon_workflow::WriteBoundaryProbe for CompletionProofRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for CompletionProofRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{
                        "items": [
                            {"wave":"wave3","target_files":["src/provider.rs"],"task":"edit T040"}
                        ],
                        "completed_items": [
                            {
                                "wave":"wave1",
                                "task_ids":["T001"],
                                "status":"already_implemented",
                                "verified":true,
                                "evidence":[
                                    {"path":"context/activeContext.md","summary":"T001 audit maps current implementation and residual gaps"}
                                ]
                            }
                        ]
                    }"#,
                ));
            }
            self.seen.lock().unwrap().push(request.stage_id);
            Ok(StageRunOutput::markdown("unexpected item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-filter-with-proof
task: Accept fact-backed no-op waves only.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${discover.items}
    filter: item.wave == 'wave1'
    allow_empty_when_completed: true
    completion_task_ids: [T001]
    depends_on: [discover]
"#,
    )
    .unwrap();

    let runner = CompletionProofRunner {
        seen: Mutex::new(Vec::new()),
    };
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &runner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
    assert!(runner.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn empty_filtered_implementation_requires_specific_completion_task_ids() {
    struct VagueFlagRunner;

    impl archon_workflow::WriteBoundaryProbe for VagueFlagRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for VagueFlagRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{
                        "items": [
                            {"wave":"wave2","target_files":["src/core.rs"],"task":"edit T010"}
                        ],
                        "completed_items": [
                            {
                                "wave":"wave1",
                                "task_ids":["T001"],
                                "status":"already_implemented",
                                "verified":true,
                                "evidence":[
                                    {"path":"context/activeContext.md","summary":"T001 audit maps current implementation"}
                                ]
                            }
                        ]
                    }"#,
                ));
            }
            Ok(StageRunOutput::markdown("unexpected item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-filter-vague-proof-flag
task: Reject vague completion flags.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${discover.items}
    filter: item.wave == 'wave1'
    allow_empty_when_completed: true
    depends_on: [discover]
"#,
    )
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &VagueFlagRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Failed);
    assert!(
        stage
            .error
            .as_deref()
            .unwrap()
            .contains("completion_task_ids")
    );
}
