use std::collections::BTreeSet;

use archon_workflow::{
    FinalizationRecordV1, ObserverAuthority, RunEndAcceptanceObserverSnapshotV1,
    RunEndObserverOutcomeV1, RunEndObserverStateV1, WorkflowRunKind, WorkflowV2Status,
};

fn snapshot() -> RunEndAcceptanceObserverSnapshotV1 {
    RunEndAcceptanceObserverSnapshotV1 {
        native_execution: None,
        schema_version: 1,
        canonical_task_root_identity: "/project/tasks".into(),
        expected_artifact_paths: BTreeSet::from([
            "acceptance-contract.json".into(),
            "acceptance-contract.lock".into(),
            "acceptance-pin.json".into(),
            "task-skeleton.json".into(),
            "task-skeleton.lock".into(),
        ]),
        portable_acceptance_identity: None,
    }
}

#[test]
fn legacy_finalization_serialization_omits_observer_state() {
    let record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        None,
    );
    let json = serde_json::to_value(record).expect("record json");
    assert!(json.get("observer_state").is_none(), "{json:#}");
    assert!(json.get("observer_snapshot").is_none(), "{json:#}");
}

#[test]
fn expected_snapshot_can_omit_unreadable_portable_identity() {
    let mut launch = snapshot();
    launch.portable_acceptance_identity = None;
    let record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        Some(launch.clone()),
    );
    assert_eq!(record.observer_snapshot, Some(launch));
    assert_eq!(record.observer_state, Some(RunEndObserverStateV1::Pending));
}

#[test]
fn expected_authored_completion_persists_pending_observer_intent() {
    let snapshot = snapshot();
    let record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::NeedsReview,
        Some(snapshot.clone()),
    );
    assert_eq!(record.observer_snapshot, Some(snapshot));
    assert_eq!(record.observer_state, Some(RunEndObserverStateV1::Pending));
    assert!(!record.terminal_event_committed);
}

#[test]
fn closed_eligibility_table_excludes_fixed_and_noncompletion_states() {
    for kind in [
        WorkflowRunKind::FixedDecompositionV1,
        WorkflowRunKind::FixedOrSavedScript,
        WorkflowRunKind::LegacyDecomposed,
    ] {
        let record = FinalizationRecordV1::new(kind, WorkflowV2Status::Accepted, Some(snapshot()));
        assert!(record.observer_state.is_none(), "kind={kind:?}");
        assert!(record.observer_snapshot.is_none(), "kind={kind:?}");
    }
    for status in [
        WorkflowV2Status::Pending,
        WorkflowV2Status::Running,
        WorkflowV2Status::Blocked,
        WorkflowV2Status::Failed,
        WorkflowV2Status::Cancelled,
    ] {
        let record = FinalizationRecordV1::new(
            WorkflowRunKind::AuthoredTaskWorkflow,
            status,
            Some(snapshot()),
        );
        assert!(record.observer_state.is_none(), "status={status:?}");
    }
}

#[test]
fn observer_transition_requires_terminal_event_commit_and_stays_observe_only() {
    let mut record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        Some(snapshot()),
    );
    let outcome = RunEndObserverOutcomeV1 {
        authority: ObserverAuthority::ObserveOnly,
        evaluated_floor_count: 2,
        policy_finding_count: 1,
        operational_deferral_count: 3,
    };
    let error = record
        .complete_observer(outcome.clone())
        .expect_err("event marker is required");
    assert!(error.to_string().contains("terminal event"), "{error}");

    record.mark_terminal_event_committed();
    record.complete_observer(outcome.clone()).expect("complete");
    assert_eq!(
        record.observer_state,
        Some(RunEndObserverStateV1::Completed { outcome })
    );
}

#[test]
fn orderly_retry_can_finish_a_pending_observer_after_event_commit() {
    let mut record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Noop,
        Some(snapshot()),
    );
    record.mark_terminal_event_committed();
    let round_trip: FinalizationRecordV1 =
        serde_json::from_value(serde_json::to_value(&record).expect("json")).expect("record");
    assert_eq!(
        round_trip.observer_state,
        Some(RunEndObserverStateV1::Pending)
    );

    let mut retried = round_trip;
    retried
        .fail_observer("frozen chain was replaced".into())
        .expect("observer failure persists");
    assert_eq!(
        retried.observer_state,
        Some(RunEndObserverStateV1::Failed {
            reason: "frozen chain was replaced".into()
        })
    );
    assert_eq!(
        retried.terminal_status,
        archon_workflow::RunStatus::Completed
    );
    assert_eq!(retried.terminal_v2_status, Some(WorkflowV2Status::Noop));
}
