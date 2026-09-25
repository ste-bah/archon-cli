//! Branch reuse of a remediation verdict follows the fix it judged.

use super::*;

/// A remediation verifier branch as the host builds it: read-only, the item
/// id `{id}-check` carrying the ordinal, the verdict's completion evidence.
fn verdict_item(ordinal: u64) -> WorkflowV2FanoutItem {
    let id = format!("review-verify-task-b-1-{ordinal}");
    let call_id = format!("verification-wave-{id}");
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".to_string(),
        serde_json::json!({ "version": 1, "stage": "verify", "taskId": "TASK-B", "round": 1, "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
    );
    let call = WorkflowV2HostCall {
        id: format!("{call_id}-0"),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    };
    WorkflowV2FanoutItem::read_only(
        format!("{call_id}-0"),
        "coder",
        call,
        serde_json::json!({
            "fanout_call_id": call_id,
            "item": { "item_id": format!("{id}-check"), "canonical_task_ids": ["TASK-B"], "task": "Verify [f1]" },
        }),
    )
}

/// An earlier session: fix 31 landed, verdict 32 accepted it.
fn earlier_verdict(store: &WorkflowV2ResultStore) {
    seed(
        store,
        &remediation_item("TASK-B", 1, 31, "[f1]"),
        WorkflowV2Status::Accepted,
    );
    let earlier = WorkflowV2ResultStore::new(store.root().to_path_buf());
    let verdict = verdict_item(32);
    let call_id = fanout_call_id(&verdict);
    let mut call = verdict.call.clone();
    call.id = call_id.clone();
    earlier
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            call,
            1,
            "input".to_string(),
            result(WorkflowV2Status::Accepted, serde_json::json!({})),
            Vec::new(),
        ))
        .expect("record");
    let mut outcome = outcome_for(
        &verdict,
        WorkflowV2Status::Accepted,
        None,
        serde_json::json!({}),
    );
    outcome.completion_evidence = vec![
        crate::v2::result_store::WorkflowV2TaskCompletionEvidence::new(
            "TASK-B",
            crate::v2::result_store::WorkflowV2TaskCompletionEvidenceKind::FocusedVerification,
            call_id.clone(),
            verdict.id.clone(),
            WorkflowV2Status::Accepted,
        ),
    ];
    earlier
        .save_branch_outcome(&call_id, &outcome)
        .expect("outcome");
}

#[test]
fn a_recorded_verdict_answers_only_the_fix_it_judged() {
    let key = crate::v2::script::resume_verdict::remediation_round_key(&verdict_item(30).call)
        .expect("key");
    for (lineage, ordinal, replays) in [
        (None, 30, false),
        (Some("review-remediate-task-b-1-27"), 30, false),
        (Some("review-remediate-task-b-1-31"), 30, true),
        (None, 32, false),
        (Some("review-remediate-task-b-1-31"), 32, true),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        earlier_verdict(&store);
        store.note_fix_lineage(&key, lineage.map(str::to_string));
        let item = verdict_item(ordinal);
        let (reused, pending) =
            split_reusable_branch_outcomes(&store, &fanout_call_id(&item), vec![item])
                .expect("split");
        assert_eq!(
            (reused.len(), pending.len()),
            if replays { (1, 0) } else { (0, 1) },
            "fix lineage {lineage:?}, verdict at {ordinal}"
        );
    }
}

/// Only a sibling that could answer the branch asks the audit to refresh:
/// the refresh is charged to the run's unexpected-change allowance.
#[test]
fn only_a_sibling_that_could_answer_the_branch_earns_a_drift_identity() {
    for (status, earns) in [
        (WorkflowV2Status::NeedsReview, false),
        (WorkflowV2Status::Failed, false),
        (WorkflowV2Status::Accepted, true),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        seed(&store, &remediation_item("TASK-B", 1, 31, "[f1]"), status);
        let mut branches = vec![remediation_item("TASK-B", 1, 29, "[f1]")];
        let call_id = fanout_call_id(&branches[0]);
        stamp_drift_identities(&mut branches, &call_id, &store).expect("stamp");
        assert_eq!(has_drift_identities(&branches[0]), earns, "{status:?}");
    }
}
