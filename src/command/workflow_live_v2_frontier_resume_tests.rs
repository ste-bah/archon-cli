use archon_workflow::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2Status,
};

use archon_workflow::v2::script::frontier_resume_record_reusable;

const RECORDED_INPUT_HASH: &str = "recorded-input-hash";

fn record(status: WorkflowV2Status, scaffold: &str) -> WorkflowV2CallRecord {
    let mut result = WorkflowV2Result::accepted("accepted with evidence");
    result.status = status;
    result
        .evidence
        .push(archon_workflow::WorkflowV2Evidence::new(
            archon_workflow::WorkflowV2EvidenceKind::Inspection,
            "durable evidence",
        ));
    WorkflowV2CallRecord::new(
        "run",
        WorkflowV2HostCall {
            id: "discovery".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        RECORDED_INPUT_HASH.to_string(),
        result,
        Vec::new(),
    )
    .with_source_metadata(Some("stale-source".to_string()), None)
    .with_scaffold_hash(Some(scaffold.to_string()))
}

#[test]
fn frontier_resume_adopts_valid_accepted_record_despite_stale_source() {
    // The frontier path stays laxer than strict reuse about the dynamic source
    // fingerprint — that waiver is the reason it exists.
    assert!(frontier_resume_record_reusable(
        &record(WorkflowV2Status::Accepted, "same-scaffold"),
        RECORDED_INPUT_HASH,
        "same-scaffold"
    ));
}

#[test]
fn frontier_resume_rejects_nonaccepted_or_different_scaffold() {
    assert!(!frontier_resume_record_reusable(
        &record(WorkflowV2Status::NeedsReview, "same-scaffold"),
        RECORDED_INPUT_HASH,
        "same-scaffold"
    ));
    assert!(!frontier_resume_record_reusable(
        &record(WorkflowV2Status::Accepted, "old-scaffold"),
        RECORDED_INPUT_HASH,
        "new-scaffold"
    ));
}

#[test]
fn frontier_resume_rejects_a_record_whose_input_changed() {
    // The defect: frontier reuse ignored the input hash, so a resume replayed a
    // recorded result for a call whose upstream output had since changed. No
    // invalidation pass covers that — `invalidate_*` only runs from the
    // operator's `workflow restart` command.
    assert!(!frontier_resume_record_reusable(
        &record(WorkflowV2Status::Accepted, "same-scaffold"),
        "input-hash-after-upstream-changed",
        "same-scaffold"
    ));
}

#[test]
fn frontier_resume_rejects_an_invalidated_record() {
    let mut record = record(WorkflowV2Status::Accepted, "same-scaffold");
    record.invalidated_by = Some("restart".to_string());
    assert!(!frontier_resume_record_reusable(
        &record,
        RECORDED_INPUT_HASH,
        "same-scaffold"
    ));
}
