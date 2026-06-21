use std::path::Path;

use archon_workflow::{
    LifecycleAction, LifecycleController, StageRunOutput, StageRunRequest, StageStatus,
    WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
    WriteBoundaryProbe,
};

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(&["init", "-q", "-b", "main"], dir.path());
    git(&["config", "user.name", "t"], dir.path());
    git(&["config", "user.email", "t@local"], dir.path());
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/seed.rs"), "// seed\n").unwrap();
    git(&["add", "-A"], dir.path());
    git(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn missing_work_unit_artifact_feeds_downstream_remediation_items() {
    struct MissingThenRepairRunner;

    impl WriteBoundaryProbe for MissingThenRepairRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for MissingThenRepairRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "implement-0" => partial_t040_output(&request),
                "repair-0" => {
                    assert_eq!(
                        request.input["fanout_item"]["finding_id"].as_str(),
                        Some("missing-work-unit:T050")
                    );
                    assert_eq!(
                        request.input["fanout_item"]["work_unit_id"].as_str(),
                        Some("T050")
                    );
                    Ok(StageRunOutput::markdown("status: accepted"))
                }
                _ => Ok(StageRunOutput::markdown("status: accepted")),
            }
        }
    }

    let repo = canonical_repo();
    let target = repo.path().join("src/serial.rs");
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = remediation_feed_spec(&target);

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &MissingThenRepairRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished.stages.get("remediation-inventory").unwrap().status,
        StageStatus::Accepted
    );
    assert!(finished.items.contains_key("repair-0"));
}

#[tokio::test]
async fn missing_work_unit_attempt_cap_blocks_after_restart() {
    struct PartialDirectRunner {
        target: String,
    }

    impl WriteBoundaryProbe for PartialDirectRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for PartialDirectRunner {
        async fn run_stage(
            &self,
            _request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            std::fs::write(&self.target, "// partial\n").unwrap();
            Ok(StageRunOutput::markdown(
                serde_json::json!({
                    "status": "implemented",
                    "implemented_work_unit_ids": ["docs-linux-install"],
                    "changed_files": [&self.target],
                    "commands_run": [{
                        "role": "verification",
                        "command": "generic verify docs-linux-install",
                        "exit_status": 0
                    }],
                    "residual_gaps": []
                })
                .to_string(),
            ))
        }
    }

    let repo = canonical_repo();
    let target = repo.path().join("src/direct-cap.rs");
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            missing_unit_remediation_max_attempts: 1,
            ..WorkflowPolicy::default()
        },
    );
    let run = executor.start(direct_cap_spec(&target)).unwrap();

    let first = executor
        .execute_with_runner(
            run.clone(),
            &PartialDirectRunner {
                target: target.display().to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(first.failed, 1);

    let restarted = LifecycleController::new(store.clone())
        .apply(&run.id, LifecycleAction::RestartStage("implement".into()))
        .unwrap();
    let second = executor
        .execute_with_runner(
            restarted,
            &PartialDirectRunner {
                target: target.display().to_string(),
            },
        )
        .await
        .unwrap();

    assert_eq!(second.blocked, 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Blocked
    );
    let body = missing_unit_body(&store, &run.id, "implement");
    assert!(body.contains(r#""attempts_exhausted": true"#), "{body}");
    assert!(body.contains(r#""blocked_work_units": ["#), "{body}");
}

fn partial_t040_output(
    request: &StageRunRequest,
) -> archon_workflow::WorkflowResult<StageRunOutput> {
    let target = request.input["fanout_item"]["target_files"][0]
        .as_str()
        .unwrap();
    std::fs::create_dir_all(Path::new(target).parent().unwrap()).unwrap();
    std::fs::write(target, "// partial\n").unwrap();
    Ok(StageRunOutput::markdown(
        serde_json::json!({
            "status": "implemented",
            "implemented_task_ids": ["T040"],
            "changed_files": [target],
            "commands_run": [{
                "role": "verification",
                "command": "generic verify T040",
                "exit_status": 0
            }],
            "residual_gaps": []
        })
        .to_string(),
    ))
}

fn remediation_feed_spec(target: &Path) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: missing-unit-remediation-feed
task: Missing work units become repair work.
stages:
  - id: implement
    kind: fanout
    item_kind: implementation
    completion_task_ids: [T040, T050]
    input:
      items:
        - task_id: T040
          target_files:
            - "{}"
  - id: remediation-inventory
    kind: reduce
    outputs: [items]
    depends_on: [implement]
  - id: repair
    kind: fanout
    foreach: "${{remediation-inventory.items}}"
    depends_on: [remediation-inventory]
"#,
        target.display()
    ))
    .unwrap()
}

fn direct_cap_spec(target: &Path) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: direct-work-unit-cap
task: Missing direct work units block after the configured cap.
stages:
  - id: implement
    kind: implementation
    agent: workflow-coder
    required_work_units: [docs-linux-install, checkout-ui]
    expected_target_files:
      - "{}"
"#,
        target.display()
    ))
    .unwrap()
}

fn missing_unit_body(store: &WorkflowStore, run_id: &str, stage_id: &str) -> String {
    let run = store.load_state(run_id).unwrap();
    run.stages
        .get(stage_id)
        .unwrap()
        .artifacts
        .iter()
        .find(|artifact| {
            artifact
                .path
                .to_string_lossy()
                .contains("missing_work_unit_remediation")
        })
        .map(|artifact| {
            std::fs::read_to_string(store.run_dir(run_id).join(&artifact.path)).unwrap()
        })
        .expect("missing-unit remediation artifact")
}
