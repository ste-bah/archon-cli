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
        lineage_recording: None,
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

/// ACC-A9 (was: the transition required the terminal event commit): the
/// observation finishes on the uncommitted record, before the commit, and the
/// observer stays observe-only. A finished observation cannot finish again.
#[test]
fn observer_transition_precedes_terminal_commit_and_stays_observe_only() {
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
    assert!(!record.terminal_event_committed);
    record.complete_observer(outcome.clone()).expect("complete");
    assert_eq!(
        record.observer_state,
        Some(RunEndObserverStateV1::Completed {
            outcome: outcome.clone()
        })
    );
    let error = record
        .complete_observer(outcome)
        .expect_err("only a pending observation finishes");
    assert!(error.to_string().contains("observer_pending"), "{error}");
}

/// A failed pre-commit observation is reopened in place while the outcome is
/// re-decided; neither is possible once the terminal event is committed.
#[test]
fn a_pre_commit_failure_reopens_and_restates_until_the_commit() {
    let mut record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        Some(snapshot()),
    );
    record
        .reopen_before_commit("chain differs".into())
        .expect("a pending observation reopens");
    assert_eq!(
        record.prior_observer_failures,
        vec!["chain differs".to_string()]
    );
    assert_eq!(record.observer_state, Some(RunEndObserverStateV1::Pending));
    let blocking = archon_workflow::AuthoredAcceptanceGateV1 {
        final_round: 1,
        attempt: 2,
        record_path: "v2/acceptance/round-01/attempt-02.json".into(),
        contract_present: true,
        failing_check_ids: vec!["REQ-1".into()],
        unowned_failing_check_ids: Vec::new(),
        operational_errors: Vec::new(),
    };
    assert!(
        record
            .restate(WorkflowV2Status::Accepted, Some(blocking.clone()))
            .is_err(),
        "a blocking gate never sits beside a completing status"
    );
    record
        .restate(WorkflowV2Status::NeedsReview, Some(blocking.clone()))
        .expect("restated");
    assert_eq!(
        record.terminal_status,
        archon_workflow::RunStatus::NeedsReview
    );
    assert_eq!(record.acceptance_gate, Some(blocking));
    assert!(
        record.restate(WorkflowV2Status::Failed, None).is_err(),
        "the pending observation stays armed"
    );
    record.fail_observer("chain differs".into()).unwrap();
    assert!(record.reopen_before_commit("again".into()).is_err());
    record.mark_terminal_event_committed();
    assert!(record.restate(WorkflowV2Status::NeedsReview, None).is_err());
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

#[test]
fn only_a_failed_or_interrupted_reopened_observer_can_be_reopened() {
    let mut record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        Some(snapshot()),
    );
    assert!(
        record.reopen_observer().is_err(),
        "not before the terminal event"
    );
    record.mark_terminal_event_committed();
    assert!(
        record.reopen_observer().is_err(),
        "the finalizer's own pending observation is not reopened"
    );
    record.fail_observer("chain differs".into()).unwrap();
    assert_eq!(record.reopen_observer().unwrap(), "chain differs");
    assert_eq!(record.observer_state, Some(RunEndObserverStateV1::Pending));
    assert_eq!(
        record.prior_observer_failures,
        vec!["chain differs".to_string()]
    );
    record
        .reopen_observer()
        .expect("an interrupted re-observation reopens");
    assert_eq!(record.prior_observer_failures.len(), 2);
    record
        .complete_observer(RunEndObserverOutcomeV1 {
            authority: ObserverAuthority::ObserveOnly,
            evaluated_floor_count: 1,
            policy_finding_count: 0,
            operational_deferral_count: 0,
        })
        .unwrap();
    assert!(
        record.reopen_observer().is_err(),
        "a completed observation stays"
    );
    let legacy: FinalizationRecordV1 = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "run_kind": "authored_task_workflow", "terminal_status": "completed",
        "terminal_state_committed": true, "terminal_event_committed": true
    }))
    .unwrap();
    assert!(legacy.prior_observer_failures.is_empty());
}
