use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};
use std::sync::Mutex;

#[tokio::test]
async fn empty_filtered_implementation_accepts_audit_only_completion_proof() {
    struct AuditOnlyProofRunner;

    impl archon_workflow::WriteBoundaryProbe for AuditOnlyProofRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for AuditOnlyProofRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"items:
  - wave: wave3
    task_ids: [T040]
    target_files: [src/provider.rs]
    task: edit T040
completed_items:
  - wave: wave1
    task_ids: [T001]
    status: completed_audit_only
    verified: true
    evidence:
      - path: tasks/context/progress.md
        summary: T001 is an audit-only task with no repository write required
"#,
                ));
            }
            Ok(StageRunOutput::markdown("unexpected item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = executor(store.clone());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-filter-audit-only-proof
task: Accept fact-backed audit-only no-op waves.
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
        .execute_with_runner(run.clone(), &AuditOnlyProofRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn completed_scope_allows_matching_downstream_review_fanout_to_noop() {
    struct DownstreamNoopRunner {
        seen: Mutex<Vec<String>>,
    }

    impl archon_workflow::WriteBoundaryProbe for DownstreamNoopRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for DownstreamNoopRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{
                        "items": [
                            {"wave":"wave3","task_ids":["T040"],"target_files":["src/provider.rs"],"task":"edit T040"}
                        ],
                        "completed_items": [
                            {
                                "wave":"wave1",
                                "task_ids":["T001"],
                                "status":"completed_audit_only",
                                "verified":true,
                                "evidence":[
                                    {"path":"tasks/context/progress.md","summary":"T001 audit-only task has concrete completed evidence"}
                                ]
                            }
                        ]
                    }"#,
                ));
            }
            self.seen.lock().unwrap().push(request.stage_id);
            Ok(StageRunOutput::markdown("unexpected fanout item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = executor(store.clone());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: completed-scope-downstream-noop
task: Completed implementation scope should not break downstream reviews.
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
  - id: review
    kind: fanout
    foreach: ${discover.items}
    filter: item.wave == 'wave1'
    depends_on: [discover, implement]
"#,
    )
    .unwrap();

    let runner = DownstreamNoopRunner {
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
        finished.stages.get("review").unwrap().status,
        StageStatus::Accepted
    );
    assert!(runner.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn empty_completed_scope_allows_downstream_review_without_filter_to_noop() {
    struct EmptyScopeRunner {
        seen: Mutex<Vec<String>>,
    }

    impl archon_workflow::WriteBoundaryProbe for EmptyScopeRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for EmptyScopeRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "inventory" {
                return Ok(StageRunOutput::markdown(
                    r#"{
                        "items": [],
                        "completed_items": [{
                            "task_ids": ["TASK-TDL-001"],
                            "canonical_task_ids": ["T001-data-lake-gap-audit"],
                            "status": "accepted",
                            "verified": true,
                            "evidence": [
                                "tasks/context/progress.md marks T001 audit-only work complete.",
                                "activeContext.md records concrete no-edit evidence."
                            ]
                        }]
                    }"#,
                ));
            }
            self.seen.lock().unwrap().push(request.stage_id);
            Ok(StageRunOutput::markdown("unexpected fanout item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = executor(store.clone());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: empty-completed-scope-review-noop
task: Completed empty implementation scope should not break downstream review fanouts.
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${inventory.items}
    allow_empty_when_completed: true
    completion_task_ids: [T001]
    depends_on: [inventory]
  - id: review
    kind: fanout
    foreach: ${inventory.items}
    depends_on: [inventory, implement]
"#,
    )
    .unwrap();

    let runner = EmptyScopeRunner {
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
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Accepted
    );
    assert!(runner.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn completed_scope_accepts_concrete_evidence_alias_from_discovery() {
    struct AliasProofRunner;

    impl archon_workflow::WriteBoundaryProbe for AliasProofRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for AliasProofRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{
                        "items": [
                            {"wave":"wave2","task_ids":["T010"],"target_files":["src/provider.rs"],"task":"edit T010"}
                        ],
                        "completed_items": [
                            {
                                "wave":"wave1",
                                "task_ids":["TASK-TDL-001"],
                                "status":"completed_audit_only",
                                "verified":true,
                                "concrete_evidence":[
                                    "tasks/context/progress.md marks TASK-TDL-001 complete.",
                                    "activeContext.md records no repository edit is required."
                                ]
                            }
                        ]
                    }"#,
                ));
            }
            Ok(StageRunOutput::markdown("unexpected fanout item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = executor(store.clone());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: completed-scope-concrete-evidence-alias
task: Discovery aliases should still prove completed scopes.
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
        .execute_with_runner(run.clone(), &AliasProofRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn completed_scope_accepts_verified_accepted_status() {
    struct AcceptedProofRunner;

    impl archon_workflow::WriteBoundaryProbe for AcceptedProofRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for AcceptedProofRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{
                        "items": [],
                        "completed_items": [{
                            "task_ids": ["TASK-TDL-001"],
                            "canonical_task_ids": ["T001-data-lake-gap-audit"],
                            "status": "accepted",
                            "verified": true,
                            "evidence": [
                                "tasks/context/progress.md marks T001 as audit complete.",
                                "activeContext.md records concrete no-edit evidence for T001."
                            ]
                        }]
                    }"#,
                ));
            }
            Ok(StageRunOutput::markdown("unexpected item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = executor(store.clone());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: accepted-status-proof
task: Accepted no-op status with evidence should satisfy completed scope.
stages:
  - id: discover
    kind: agent
    outputs: [items]
  - id: implement
    kind: fanout
    item_kind: implementation
    foreach: ${discover.items}
    allow_empty_when_completed: true
    completion_task_ids: [T001]
    depends_on: [discover]
"#,
    )
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &AcceptedProofRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn negative_completion_status_is_not_a_valid_empty_wave_proof() {
    struct NegativeProofRunner;

    impl archon_workflow::WriteBoundaryProbe for NegativeProofRunner {}

    #[async_trait::async_trait]
    impl WorkflowStageRunner for NegativeProofRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "discover" {
                return Ok(StageRunOutput::markdown(
                    r#"{
                        "items": [{"wave":"wave2","target_files":["src/lib.rs"],"task":"edit T010"}],
                        "completed_items": [{
                            "wave":"wave1",
                            "task_ids":["T001"],
                            "status":"not_already_implemented",
                            "verified":true,
                            "evidence":[{"path":"tasks/context/progress.md","summary":"negative proof must not pass"}]
                        }]
                    }"#,
                ));
            }
            Ok(StageRunOutput::markdown("unexpected item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = executor(store.clone());
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: negative-proof-status
task: Reject negative completion statuses.
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
        .execute_with_runner(run.clone(), &NegativeProofRunner)
        .await
        .unwrap();
    let finished = store.load_state(&run.id).unwrap();

    assert_eq!(report.failed, 1);
    assert_eq!(
        finished.stages.get("discover").unwrap().status,
        StageStatus::Failed
    );
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::Pending
    );
}

fn executor(store: WorkflowStore) -> WorkflowExecutor {
    WorkflowExecutor::new(
        store,
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    )
}
