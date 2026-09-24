use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use super::workflow_live_v2_script::WorkflowV2ScriptSummary;
use archon_workflow::{
    FinalizationRecordV1, ObserverAuthority, RunEndAcceptanceObserverSnapshotV1,
    RunEndObserverOutcomeV1, RunEndObserverStateV1, StageStatus, WorkflowEvent, WorkflowRunKind,
    WorkflowSpec,
};

use super::workflow_live_v2_finalizer::{
    FINALIZATION_RECORD_PATH, RunEndObserverContext, WorkflowRunEndObserver, finalize_run_status,
    finalize_summary,
};

pub(super) fn spec() -> WorkflowSpec {
    WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "finalizer-test".into(),
        task: "prove terminal ordering".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: vec![archon_workflow::StageSpec {
            id: "call-1".into(),
            kind: archon_workflow::StageKind::Agent,
            task: Some("test".into()),
            agent: None,
            foreach: None,
            reducer: None,
            tool: None,
            depends_on: Vec::new(),
            provider_tier: None,
            retry: Default::default(),
            input: serde_json::Value::Null,
            model: None,
            provider: None,
            expected_target_files: Vec::new(),
            verify_command: None,
            max_parallelism: None,
            item_kind: None,
            filter: None,
            extra: Default::default(),
        }],
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    }
}

pub(super) fn summary(status: WorkflowV2Status) -> WorkflowV2ScriptSummary {
    WorkflowV2ScriptSummary {
        status,
        completed: 1,
        executed: 1,
        reused: 0,
        calls: vec![WorkflowV2HostCall {
            id: "call-1".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        }],
        failed_call: None,
        failed_result_path: None,
        next_action: None,
        script_result: None,
    }
}

pub(super) fn snapshot(root: &std::path::Path) -> RunEndAcceptanceObserverSnapshotV1 {
    RunEndAcceptanceObserverSnapshotV1 {
        native_execution: None,
        schema_version: 1,
        canonical_task_root_identity: root.display().to_string(),
        expected_artifact_paths: archon_workflow::RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS
            .into_iter()
            .map(str::to_string)
            .collect(),
        portable_acceptance_identity: None,
    }
}

pub(super) fn seed_call(v2_store: &WorkflowV2ResultStore, status: WorkflowV2Status) {
    let result = WorkflowV2Result {
        status,
        summary: "terminal call".into(),
        ..WorkflowV2Result::default()
    };
    let record = WorkflowV2CallRecord::new(
        v2_store.run_id(),
        summary(status).calls[0].clone(),
        1,
        "input".into(),
        result,
        Vec::new(),
    );
    v2_store.save_call_record(&record).expect("call record");
}

pub(super) fn read_finalization(store: &WorkflowStore, run_id: &str) -> FinalizationRecordV1 {
    serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join(FINALIZATION_RECORD_PATH)).expect("record"),
    )
    .expect("record json")
}

pub(super) fn events(store: &WorkflowStore, run_id: &str) -> Vec<WorkflowEvent> {
    std::fs::read_to_string(store.events_path(run_id))
        .expect("events")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect()
}

struct OrderingObserver {
    store: WorkflowStore,
    run_id: String,
    calls: AtomicUsize,
    fail: bool,
}

impl WorkflowRunEndObserver for OrderingObserver {
    fn observe(
        &self,
        context: &RunEndObserverContext<'_>,
    ) -> archon_workflow::WorkflowResult<RunEndObserverOutcomeV1> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(context.run_id, self.run_id);
        let run = self.store.load_state(&self.run_id).expect("terminal state");
        assert_eq!(run.status, RunStatus::Completed);
        assert_eq!(
            run.stages.get("call-1").expect("stage").status,
            StageStatus::Accepted
        );
        let record = read_finalization(&self.store, &self.run_id);
        assert!(record.terminal_state_committed);
        assert!(record.terminal_event_committed);
        assert_eq!(record.observer_state, Some(RunEndObserverStateV1::Pending));
        assert!(events(&self.store, &self.run_id).iter().any(|event| {
            event.detail.get("event") == Some(&serde_json::json!("terminal_status"))
        }));
        if self.fail {
            return Err(WorkflowError::StageFailed("observer probe failed".into()));
        }
        Ok(RunEndObserverOutcomeV1 {
            authority: ObserverAuthority::ObserveOnly,
            evaluated_floor_count: 1,
            policy_finding_count: 0,
            operational_deferral_count: 0,
        })
    }
}

#[test]
fn no_summary_terminal_paths_persist_state_record_and_event() {
    for status in [RunStatus::Paused, RunStatus::Cancelled, RunStatus::Failed] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store.create_run(spec()).unwrap();

        finalize_run_status(
            &store,
            &run.id,
            WorkflowRunKind::AuthoredTaskWorkflow,
            status.clone(),
            "terminal detail",
            None,
        )
        .unwrap();

        assert_eq!(store.load_state(&run.id).unwrap().status, status);
        let record = read_finalization(&store, &run.id);
        assert_eq!(record.terminal_status, status);
        assert!(record.terminal_v2_status.is_none());
        assert!(record.terminal_state_committed);
        assert!(record.terminal_event_committed);
        assert_eq!(
            events(&store, &run.id)
                .iter()
                .filter(|event| event.detail["event"] == "terminal_status")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn finalizer_persists_state_then_event_then_observer_completion() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);
    let observer = OrderingObserver {
        store: store.clone(),
        run_id: run.id.clone(),
        calls: AtomicUsize::new(0),
        fail: false,
    };

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(snapshot(temp.path())),
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        Some(&observer),
        None,
    )
    .await
    .unwrap();

    assert_eq!(observer.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        read_finalization(&store, &run.id).observer_state,
        Some(RunEndObserverStateV1::Completed { .. })
    ));
}

#[tokio::test]
async fn observer_failure_is_post_terminal_and_never_changes_run_status() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);
    let observer = OrderingObserver {
        store: store.clone(),
        run_id: run.id.clone(),
        calls: AtomicUsize::new(0),
        fail: true,
    };

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(snapshot(temp.path())),
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        Some(&observer),
        None,
    )
    .await
    .unwrap();

    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Completed
    );
    assert!(matches!(
        read_finalization(&store, &run.id).observer_state,
        Some(RunEndObserverStateV1::Failed { .. })
    ));
    let terminal_index = events(&store, &run.id)
        .iter()
        .position(|event| event.detail["event"] == "terminal_status")
        .unwrap();
    let observer_index = events(&store, &run.id)
        .iter()
        .position(|event| event.detail["event"] == "run_end_acceptance_observer_failed")
        .unwrap();
    assert!(terminal_index < observer_index);
}

#[tokio::test]
async fn omitted_legacy_snapshot_is_observer_silent() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);
    let observer = OrderingObserver {
        store: store.clone(),
        run_id: run.id.clone(),
        calls: AtomicUsize::new(0),
        fail: false,
    };

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        Some(&observer),
        None,
    )
    .await
    .unwrap();

    assert_eq!(observer.calls.load(Ordering::SeqCst), 0);
    let record = read_finalization(&store, &run.id);
    assert!(record.observer_state.is_none());
    assert!(events(&store, &run.id).iter().all(|event| {
        !event.detail["event"]
            .as_str()
            .unwrap_or_default()
            .starts_with("run_end_acceptance")
    }));
}

#[tokio::test]
async fn fixed_decomposition_cannot_create_observer_intent() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);
    let observer = OrderingObserver {
        store: store.clone(),
        run_id: run.id.clone(),
        calls: AtomicUsize::new(0),
        fail: false,
    };

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::FixedDecompositionV1,
        Some(snapshot(temp.path())),
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        Some(&observer),
        None,
    )
    .await
    .unwrap();

    assert_eq!(observer.calls.load(Ordering::SeqCst), 0);
    assert!(read_finalization(&store, &run.id).observer_state.is_none());
}

#[tokio::test]
async fn orderly_retry_finishes_pending_observer_without_duplicate_terminal_event() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2_store, WorkflowV2Status::Accepted);
    let mut pending = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        Some(snapshot(temp.path())),
    );
    pending.mark_terminal_event_committed();
    archon_workflow::v2::run_state_sync::sync_v2_summary_to_run(
        &store,
        &run.id,
        &summary(WorkflowV2Status::Accepted).calls,
        &v2_store,
        WorkflowV2Status::Accepted,
    )
    .unwrap();
    store
        .write_run_json(&run.id, FINALIZATION_RECORD_PATH, &pending)
        .unwrap();
    WorkflowEventLog::new(store.clone())
        .emit(
            &run.id,
            1,
            WorkflowEventKind::StageCompleted,
            serde_json::json!({"event": "terminal_status", "status": "accepted"}),
        )
        .unwrap();
    let observer = OrderingObserver {
        store: store.clone(),
        run_id: run.id.clone(),
        calls: AtomicUsize::new(0),
        fail: false,
    };

    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(snapshot(temp.path())),
        &summary(WorkflowV2Status::Accepted),
        &v2_store,
        Some(&observer),
        None,
    )
    .await
    .unwrap();

    assert_eq!(observer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        events(&store, &run.id)
            .iter()
            .filter(|event| event.detail["event"] == "terminal_status")
            .count(),
        1
    );
}

#[tokio::test]
async fn repository_audit_open_finding_prevents_terminal_success() {
    use archon_workflow::repository_audit::{
        AuditContract, AuditReport,
        budget::{AuditPolicy, Limit},
        runtime::{AuditRuntime, Snapshot},
    };
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2, WorkflowV2Status::Accepted);
    let audit = AuditRuntime::initialize(
        store.clone(),
        run.id.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    audit.update(|s| {
        s.snapshot = Some(Snapshot{identity:"sealed".into(),root:temp.path().into(),paths:vec![]});
        let report: AuditReport = serde_json::from_value(serde_json::json!({"schema_version":1,"snapshot":"sealed","records":[{
            "declared_path":"new.txt","verdict":"exists_elsewhere","equivalents":["old.txt"],"required_action":"wire_or_migrate","reason":"existing implementation"
        }]})).unwrap();
        s.ledger.accept(AuditContract{schema_version:1,snapshot:"sealed".into(),declared_paths:vec!["new.txt".into()]},report)
    }).unwrap();
    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::FixedOrSavedScript,
        None,
        &summary(WorkflowV2Status::Accepted),
        &v2,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Failed);
    assert!(
        !events(&store, &run.id)
            .iter()
            .any(|e| e.detail["event"] == "terminal_status" && e.detail["status"] == "accepted")
    );
}

#[tokio::test]
async fn repository_audit_clean_dispatch_is_not_a_final_assessment_receipt() {
    use archon_workflow::repository_audit::{
        AuditContract, AuditReport,
        budget::{AuditPolicy, Limit},
        runtime::{AuditRuntime, Snapshot},
    };
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    seed_call(&v2, WorkflowV2Status::Accepted);
    let audit = AuditRuntime::initialize(
        store.clone(),
        run.id.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        },
    )
    .unwrap();
    audit
        .update(|state| {
            state.snapshot = Some(Snapshot {
                identity: "dispatch".into(),
                root: temp.path().into(),
                paths: vec![],
            });
            state.ledger.accept(
                AuditContract {
                    schema_version: 1,
                    snapshot: "dispatch".into(),
                    declared_paths: vec![],
                },
                AuditReport {
                    schema_version: 1,
                    snapshot: "dispatch".into(),
                    records: vec![],
                },
            )
        })
        .unwrap();
    finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::FixedOrSavedScript,
        None,
        &summary(WorkflowV2Status::Accepted),
        &v2,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        RunStatus::Failed,
        "an earlier clean assessment was promoted to final acceptance without a final snapshot receipt"
    );
}
