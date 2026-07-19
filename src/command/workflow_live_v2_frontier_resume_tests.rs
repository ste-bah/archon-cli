use archon_workflow::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2Status,
};

use super::workflow_live_v2_script::frontier_resume_record_reusable;

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
        "stale-input-hash".to_string(),
        result,
        Vec::new(),
    )
    .with_source_metadata(Some("stale-source".to_string()), None)
    .with_scaffold_hash(Some(scaffold.to_string()))
}

#[test]
fn frontier_resume_adopts_valid_accepted_record_despite_stale_source() {
    assert!(frontier_resume_record_reusable(
        &record(WorkflowV2Status::Accepted, "same-scaffold"),
        "same-scaffold"
    ));
}

#[test]
fn frontier_resume_rejects_nonaccepted_or_different_scaffold() {
    assert!(!frontier_resume_record_reusable(
        &record(WorkflowV2Status::NeedsReview, "same-scaffold"),
        "same-scaffold"
    ));
    assert!(!frontier_resume_record_reusable(
        &record(WorkflowV2Status::Accepted, "old-scaffold"),
        "new-scaffold"
    ));
}
