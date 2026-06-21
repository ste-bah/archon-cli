use archon_workflow::{
    StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn reduce_inventory_outputs_parseable_remediation_items() {
    struct FindingRunner;

    impl archon_workflow::WriteBoundaryProbe for FindingRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for FindingRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "review" => Ok(StageRunOutput::markdown(
                    r#"{"findings":[{"finding_id":"F-1","severity":"high","target_files":["src/lib.rs"],"failure":"missing guard","required_fix":"add guard","required_tests":["cargo test -p demo"]}]}"#,
                )),
                "remediate-0" => {
                    assert_eq!(
                        request.input["fanout_item"]["finding_id"].as_str(),
                        Some("F-1")
                    );
                    Ok(StageRunOutput::markdown("status: completed"))
                }
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: reduce-remediation-items
task: Convert review findings into repair work.
stages:
  - id: review
    kind: agent
  - id: remediation-inventory
    kind: reduce
    outputs: [items]
    depends_on: [review]
  - id: remediate
    kind: fanout
    foreach: "${remediation-inventory.items}"
    depends_on: [remediation-inventory]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &FindingRunner)
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
async fn accepted_remediation_with_verification_evidence_completes() {
    struct VerificationEvidenceRunner;

    impl archon_workflow::WriteBoundaryProbe for VerificationEvidenceRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for VerificationEvidenceRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "review" => Ok(StageRunOutput::markdown(
                    r#"{"findings":[{"finding_id":"F-1","severity":"high","target_files":["src/lib.rs"],"failure":"missing guard","required_fix":"add guard","required_tests":["cargo test -p demo"]}]}"#,
                )),
                "remediate-0" => Ok(StageRunOutput::markdown(
                    r#"{
                      "status":"accepted",
                      "changed_files":["src/lib.rs"],
                      "verification":[{"command":"cargo test -p demo","result":"passed"}],
                      "residual_gaps":[]
                    }"#,
                )),
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: accepted-remediation-verification
task: Accepted remediation evidence must not be rejected.
stages:
  - id: review
    kind: agent
  - id: remediation-inventory
    kind: reduce
    outputs: [items]
    depends_on: [review]
  - id: remediate
    kind: fanout
    foreach: "${remediation-inventory.items}"
    depends_on: [remediation-inventory]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &VerificationEvidenceRunner)
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
async fn failed_remediation_stage_is_not_forced_accepted_by_later_tests() {
    struct FailedRemediationRunner;

    impl archon_workflow::WriteBoundaryProbe for FailedRemediationRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for FailedRemediationRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "inventory" => Ok(StageRunOutput::markdown(
                    r#"{"items":[{"target_files":["src/lib.rs"],"task":"repair"}]}"#,
                )),
                "implement-remediation-0" => Ok(StageRunOutput::markdown(r#"{"status":"failed"}"#)),
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: failed-remediation-stops
task: Failed remediation must stop before tests.
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: implement-remediation
    kind: fanout
    foreach: "${inventory.items}"
    item_kind: implementation
    depends_on: [inventory]
  - id: post-remediation-tests
    kind: agent
    depends_on: [implement-remediation]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &FailedRemediationRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("implement-remediation").unwrap().status,
        StageStatus::Failed
    );
    assert_eq!(
        finished
            .stages
            .get("post-remediation-tests")
            .unwrap()
            .status,
        StageStatus::Pending
    );
}

#[tokio::test]
async fn empty_live_inventory_is_repaired_from_forced_accepted_upstream_items() {
    struct EmptyInventoryRunner {
        target: String,
    }

    impl archon_workflow::WriteBoundaryProbe for EmptyInventoryRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyInventoryRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "verification" => Ok(StageRunOutput::markdown(format!(
                    r#"{{
                      "status":"failed",
                      "items":[{{
                        "finding_id":"F-1",
                        "target_files":["{}"],
                        "failure":"verification found missing implementation",
                        "required_fix":"write the missing file",
                        "required_tests":["cargo test -p demo focused"]
                      }}]
                    }}"#,
                    self.target
                ))),
                "remediation-inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                "repair-0" => {
                    assert_eq!(
                        request.input["fanout_item"]["finding_id"].as_str(),
                        Some("F-1")
                    );
                    std::fs::write(&self.target, "// repaired\n").unwrap();
                    Ok(StageRunOutput::markdown(format!(
                        r#"{{
                          "status":"accepted",
                          "implemented_task_ids":["F-1"],
                          "changed_files":["{}"],
                          "commands_run":[{{"command":"cargo test -p demo focused","exit_status":0}}],
                          "residual_gaps":[]
                        }}"#,
                        self.target
                    )))
                }
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("src/lib.rs");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: empty-inventory-repaired
task: Empty inventory must use upstream repair evidence.
stages:
  - id: verification
    kind: agent
  - id: remediation-inventory
    kind: agent
    outputs: [items]
    failure_aware: true
    depends_on: [verification]
  - id: repair
    kind: fanout
    foreach: "${{remediation-inventory.items}}"
    item_kind: implementation
    depends_on: [remediation-inventory]
"#
    ))
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(
            run.clone(),
            &EmptyInventoryRunner {
                target: target.display().to_string(),
            },
        )
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("verification").unwrap().status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished.stages.get("remediation-inventory").unwrap().status,
        StageStatus::Accepted
    );
    assert!(finished.items.contains_key("repair-0"));
}

#[tokio::test]
async fn empty_live_inventory_names_unresolved_forced_failure_without_items() {
    struct EmptyInventoryRunner;

    impl archon_workflow::WriteBoundaryProbe for EmptyInventoryRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyInventoryRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "verification" => Ok(StageRunOutput::markdown(r#"{"status":"failed"}"#)),
                "remediation-inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                other => panic!("stage `{other}` should not run"),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-inventory-fails-clearly
task: Empty inventory must not hide unresolved failures.
stages:
  - id: verification
    kind: agent
  - id: remediation-inventory
    kind: agent
    outputs: [items]
    failure_aware: true
    depends_on: [verification]
  - id: repair
    kind: fanout
    foreach: "${remediation-inventory.items}"
    item_kind: implementation
    depends_on: [remediation-inventory]
"#,
    )
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyInventoryRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let inventory = finished.stages.get("remediation-inventory").unwrap();
    assert_eq!(inventory.status, StageStatus::Failed);
    let error = inventory.error.as_deref().unwrap_or_default();
    assert!(error.contains("empty remediation inventory"), "{error}");
    assert!(error.contains("verification"), "{error}");
    let events = std::fs::read_to_string(store.run_dir(&run.id).join("events.jsonl")).unwrap();
    assert!(events.contains(r#""reason":"#), "{events}");
    assert!(events.contains("empty remediation inventory"), "{events}");
}
