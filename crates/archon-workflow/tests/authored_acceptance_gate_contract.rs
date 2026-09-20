//! The authored lifecycle's acceptance gate (Obs-32), as a contract.
//!
//! Separate from `finalization_contract.rs` on purpose: the R2 observer
//! contract there is frozen, and this gate is the authored lifecycle's own
//! rule layered beside it.

use archon_workflow::{
    AuthoredAcceptanceGateV1, FinalizationRecordV1, RunStatus, WorkflowRunKind, WorkflowV2Status,
};

fn gate(failing: &[&str]) -> AuthoredAcceptanceGateV1 {
    AuthoredAcceptanceGateV1 {
        final_round: 2,
        attempt: 1,
        record_path: "v2/acceptance/round-02/attempt-01.json".into(),
        contract_present: true,
        failing_check_ids: failing.iter().map(|id| id.to_string()).collect(),
        unowned_failing_check_ids: Vec::new(),
        operational_errors: Vec::new(),
    }
}

/// The record every other lifecycle writes is byte-for-byte what it was: no
/// gate key appears unless the authored lifecycle set one.
#[test]
fn records_without_a_gate_serialize_exactly_as_before() {
    for kind in [
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowRunKind::LegacyDecomposed,
        WorkflowRunKind::FixedDecompositionV1,
        WorkflowRunKind::FixedOrSavedScript,
    ] {
        let record = FinalizationRecordV1::new(kind, WorkflowV2Status::Accepted, None);
        let json = serde_json::to_value(&record).expect("json");
        assert!(
            json.get("acceptance_gate").is_none(),
            "kind={kind:?}: {json:#}"
        );
        let status_record = FinalizationRecordV1::for_run_status(kind, RunStatus::Paused);
        let json = serde_json::to_value(&status_record).expect("json");
        assert!(
            json.get("acceptance_gate").is_none(),
            "kind={kind:?}: {json:#}"
        );
    }
    // A record written before the field existed still reads.
    let legacy = serde_json::json!({
        "schema_version": 1,
        "run_kind": "authored_task_workflow",
        "terminal_status": "completed",
        "terminal_v2_status": "accepted",
        "terminal_state_committed": true,
        "terminal_event_committed": true
    });
    let record: FinalizationRecordV1 = serde_json::from_value(legacy).expect("legacy record");
    assert!(record.acceptance_gate.is_none());
}

/// A run whose acceptance stage recorded a failing check cannot finalize as
/// `Complete`: the record refuses the combination outright.
#[test]
fn a_failing_acceptance_gate_cannot_ride_a_completing_terminal_status() {
    for status in [WorkflowV2Status::Accepted, WorkflowV2Status::Noop] {
        let record = FinalizationRecordV1::new(WorkflowRunKind::AuthoredTaskWorkflow, status, None);
        let error = record
            .with_acceptance_gate(gate(&["REQ-2"]))
            .expect_err("a failing gate must not finalize complete");
        assert!(
            error.to_string().contains("REQ-2") && error.to_string().contains("complete"),
            "{error}"
        );
    }
    let record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        None,
    );
    let mut errored = gate(&[]);
    errored
        .operational_errors
        .push("acceptance stage could not resolve the task root".into());
    assert!(
        record.with_acceptance_gate(errored).is_err(),
        "a stage that could not evaluate is not a pass"
    );
}

#[test]
fn a_failing_gate_finalizes_as_needs_review_and_round_trips() {
    let record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::NeedsReview,
        None,
    )
    .with_acceptance_gate(gate(&["REQ-2", "REQ-3"]))
    .expect("needs review carries a failing gate");
    assert_eq!(record.terminal_status, RunStatus::NeedsReview);
    let json = serde_json::to_value(&record).expect("json");
    assert_eq!(
        json["acceptance_gate"]["failing_check_ids"],
        serde_json::json!(["REQ-2", "REQ-3"])
    );
    assert_eq!(json["acceptance_gate"]["final_round"], 2);
    let round_trip: FinalizationRecordV1 = serde_json::from_value(json).expect("record");
    assert_eq!(round_trip, record);
}

#[test]
fn a_passing_gate_finalizes_complete_and_other_lifecycles_cannot_carry_one() {
    let record = FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        None,
    )
    .with_acceptance_gate(gate(&[]))
    .expect("a clean final round completes");
    assert_eq!(record.terminal_status, RunStatus::Completed);
    assert!(!record.acceptance_gate.expect("gate").blocks_completion());
    let legacy = FinalizationRecordV1::new(
        WorkflowRunKind::LegacyDecomposed,
        WorkflowV2Status::Accepted,
        None,
    );
    assert!(legacy.with_acceptance_gate(gate(&[])).is_err());
}
