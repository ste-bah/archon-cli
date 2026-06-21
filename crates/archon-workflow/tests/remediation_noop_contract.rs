use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn empty_remediation_inventory_noops_post_remediation_chain() {
    struct EmptyRemediationRunner;

    impl archon_workflow::WriteBoundaryProbe for EmptyRemediationRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyRemediationRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "remediation_inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                other => panic!("stage `{other}` should no-op without invoking a runner"),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-post-remediation-chain
task: Empty remediation must not execute placeholder post-remediation commands.
stages:
  - id: remediation_inventory
    kind: agent
    outputs: [items]
  - id: remediation_impl
    kind: fanout
    foreach: "${remediation_inventory.items}"
    depends_on: [remediation_inventory]
    item_kind: implementation
    allow_empty_items: true
  - id: post_remediation_tests
    kind: agent
    allow_empty_remediation_noop: true
    verify_command: "${remediation_impl.focused_test_command}"
    depends_on: [remediation_impl]
  - id: post_remediation_review
    kind: fanout
    foreach: "${remediation_inventory.items}"
    depends_on: [remediation_impl, post_remediation_tests, remediation_inventory]
    allow_empty_remediation_noop: true
    allow_empty_items: true
  - id: checkpoint
    kind: checkpoint
    depends_on: [post_remediation_review]
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyRemediationRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished
            .stages
            .get("post_remediation_tests")
            .unwrap()
            .status,
        StageStatus::Accepted
    );
    assert_eq!(
        finished
            .stages
            .get("post_remediation_review")
            .unwrap()
            .status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn empty_remediation_noop_does_not_hide_forced_accepted_upstream_failure() {
    struct EmptyRemediationRunner;

    impl archon_workflow::WriteBoundaryProbe for EmptyRemediationRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyRemediationRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "remediation_inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                other => panic!("stage `{other}` should not be delegated to the runner"),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-post-remediation-preserves-upstream-failure
task: Empty remediation must not hide unresolved verification failures.
stages:
  - id: remediation_inventory
    kind: agent
    outputs: [items]
  - id: remediation_impl
    kind: fanout
    foreach: "${remediation_inventory.items}"
    depends_on: [remediation_inventory]
    item_kind: implementation
    allow_empty_items: true
  - id: verification_tests
    kind: agent
    provider_tier: local
    verify_command: "false"
    depends_on: [remediation_impl]
  - id: post_remediation_review
    kind: fanout
    foreach: "${remediation_inventory.items}"
    depends_on: [remediation_impl, verification_tests, remediation_inventory]
    allow_empty_remediation_noop: true
    allow_empty_items: true
    failure_aware: true
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyRemediationRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(
        finished.stages.get("verification_tests").unwrap().status,
        StageStatus::ForcedAccepted
    );
    let review = finished.stages.get("post_remediation_review").unwrap();
    assert_eq!(review.status, StageStatus::Failed);
    let error = review.error.as_deref().unwrap_or_default();
    assert!(
        error.contains("blocked by unresolved forced-accepted upstream stage"),
        "{error}"
    );
    assert!(error.contains("verification_tests"), "{error}");
}

#[tokio::test]
async fn empty_remediation_noop_requires_explicit_runtime_contract() {
    struct EmptyRemediationRunner;

    impl archon_workflow::WriteBoundaryProbe for EmptyRemediationRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyRemediationRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            match request.stage_id.as_str() {
                "remediation_inventory" => Ok(StageRunOutput::markdown(r#"{"items":[]}"#)),
                other => panic!("stage `{other}` should not be delegated to the runner"),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-post-remediation-requires-contract
task: Stage names alone must not authorize empty remediation no-ops.
stages:
  - id: remediation_inventory
    kind: agent
    outputs: [items]
  - id: post_remediation_review
    kind: fanout
    foreach: "${remediation_inventory.items}"
    depends_on: [remediation_inventory]
    allow_empty_items: true
"#,
    )
    .unwrap();
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &EmptyRemediationRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let review = finished.stages.get("post_remediation_review").unwrap();
    assert_eq!(review.status, StageStatus::Failed);
    let error = review.error.as_deref().unwrap_or_default();
    assert!(
        error
            .contains("resolved zero items; only explicit recovery/remediation fan-outs may no-op"),
        "{error}"
    );
}
