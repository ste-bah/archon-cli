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

fn finished(mut record: WorkflowV2CallRecord, second: u32) -> WorkflowV2CallRecord {
    record.started_at = format!("2026-01-01T00:00:{second:02}+00:00");
    record.finished_at = record.started_at.clone();
    record
}

/// Ordinals move both ways across sessions; finish time does not. The fix a
/// verdict judged is the latest fix of its unit and round that finished
/// before it, and any fix of that round finished after the replayed one
/// makes the verdict stale.
#[test]
fn a_verdict_pairs_with_the_fix_that_finished_last_before_it() {
    let store = WorkflowV2ResultStore::new(tempfile::tempdir().expect("tempdir").path().join("v2"));
    let fix = finished(record("review-remediate-task-a-1-31", "remediate", 1), 1);
    let verdict = finished(
        record("verification-wave-review-verify-task-a-1-32", "verify", 1),
        3,
    );
    let key = remediation_round_key(&verdict.call).expect("key");
    store.note_fix_lineage(&key, Some(fix.call.id.clone()));
    let alone = vec![fix.clone(), verdict.clone()];
    assert!(verdict_vouches_for_session_fix(&verdict, &alone, &store));

    // A lower-ordinal fix from a later session finished between them: the
    // verdict judged that one.
    let between = finished(record("review-remediate-task-a-1-20", "remediate", 1), 2);
    let records = vec![fix.clone(), between, verdict.clone()];
    assert!(!verdict_vouches_for_session_fix(&verdict, &records, &store));

    // A fix of the round finished after the verdict: the verdict is stale.
    let after = finished(record("review-remediate-task-a-1-40", "remediate", 1), 4);
    let records = vec![fix.clone(), verdict.clone(), after];
    assert!(!verdict_vouches_for_session_fix(&verdict, &records, &store));

    // The replayed fix itself finished after the verdict: not what it judged.
    let late = finished(record("review-remediate-task-a-1-31", "remediate", 1), 5);
    assert!(!verdict_vouches_for_session_fix(
        &verdict,
        &[late, verdict.clone()],
        &store
    ));
}
