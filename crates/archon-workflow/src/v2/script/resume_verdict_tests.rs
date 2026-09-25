use super::*;

use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn record(id: &str, stage: &str, round: u64) -> WorkflowV2CallRecord {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".to_string(),
        serde_json::json!({ "version": 1, "stage": stage, "taskId": "TASK-A", "round": round,
            "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
    );
    let mut result = WorkflowV2Result::accepted("answered");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "inspected",
    ));
    WorkflowV2CallRecord::new(
        "run",
        WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options,
        },
        1,
        "input".to_string(),
        result,
        Vec::new(),
    )
}

fn earlier_session() -> Vec<WorkflowV2CallRecord> {
    vec![
        record("review-remediate-task-a-1-27", "remediate", 1),
        record("review-remediate-task-a-1-31", "remediate", 1),
        record("verification-wave-review-verify-task-a-1-32", "verify", 1),
        record("review-remediate-task-a-2-33", "remediate", 2),
    ]
}

#[test]
fn a_verdict_is_paired_with_the_latest_fix_of_its_round_before_it() {
    let records = earlier_session();
    let paired = paired_fix(&records[2], &records, |_| false).expect("paired");
    assert_eq!(paired.call.id, "review-remediate-task-a-1-31");
}

#[test]
fn a_verdict_vouches_only_for_a_fix_replayed_from_the_fix_it_judged() {
    let records = earlier_session();
    let verdict = &records[2];
    let key = remediation_round_key(&verdict.call).expect("key");
    let store = WorkflowV2ResultStore::new(tempfile::tempdir().expect("tempdir").path().join("v2"));
    assert!(
        !verdict_vouches_for_session_fix(verdict, &records, &store),
        "no fix answered this session"
    );
    store.note_fix_lineage(&key, None);
    assert!(
        !verdict_vouches_for_session_fix(verdict, &records, &store),
        "the fix ran again"
    );
    store.note_fix_lineage(&key, Some("review-remediate-task-a-1-27".to_string()));
    assert!(
        !verdict_vouches_for_session_fix(verdict, &records, &store),
        "the fix came from another lineage"
    );
    store.note_fix_lineage(&key, Some("review-remediate-task-a-1-31".to_string()));
    assert!(verdict_vouches_for_session_fix(verdict, &records, &store));
    assert!(
        verdict_vouches_for_session_fix(&records[1], &records, &store),
        "a fix record is not a verdict"
    );
}
