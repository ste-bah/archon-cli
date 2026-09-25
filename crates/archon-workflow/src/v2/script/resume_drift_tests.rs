use super::*;

use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::v2::result_store::{
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
};
use crate::v2::scheduler::stable_value_hash;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn contract(task: &str, stage: &str, round: u64) -> Value {
    serde_json::json!({
        "version": 1, "stage": stage, "taskId": task, "round": round, "maxRounds": 2,
        "sourceReduceCallIds": ["adversarial-review-reduce", "coverage-audit-reduce"],
    })
}

/// A verifier call as the prelude issues it: the wave id, the item id and
/// the call id all carry the ordinal; the prompt carries the findings.
fn verify_execution(
    task: &str,
    round: u64,
    ordinal: u64,
    findings: &str,
) -> WorkflowV2CallExecution {
    let slug = task.to_ascii_lowercase();
    let id = format!("review-verify-{slug}-{round}-{ordinal}");
    let call_id = format!("verification-wave-{id}");
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".to_string(),
        contract(task, "verify", round),
    );
    let task_text = format!("These review findings were raised against {task}:\n{findings}");
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: call_id.clone(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        input: serde_json::json!({
            "objective": "run",
            "call_id": call_id,
            "method": "parallel",
            "write_mode": null,
            "options": { "task": task_text, "remediationContract": contract(task, "verify", round) },
            "source_data": [{ "item_id": format!("{id}-check"), "canonical_task_ids": [task], "task": task_text }],
        }),
        depends_on: Vec::new(),
    }
}

fn record_for(
    execution: &WorkflowV2CallExecution,
    status: WorkflowV2Status,
) -> WorkflowV2CallRecord {
    let mut result = WorkflowV2Result::accepted("verified the remediation");
    result.status = status;
    let label = match status {
        WorkflowV2Status::Accepted => "accepted",
        WorkflowV2Status::NeedsReview => "needs_review",
        _ => "failed",
    };
    result.data = serde_json::json!({ "outcomes": [{
        "item_id": format!("{}-0", execution.call.id), "status": label, "error": null,
        "failure_kind": if label == "needs_review" { serde_json::json!("semantic") } else { Value::Null },
        "result": { "status": label, "summary": "verifier verdict" },
    }] });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "inspected the remediated code",
    ));
    // A verifier wave's accepted record carries the completion evidence the
    // host's ledger credits; without it no path reuses one.
    let evidence = (status == WorkflowV2Status::Accepted).then(|| {
        WorkflowV2TaskCompletionEvidence::new(
            "TASK-A",
            WorkflowV2TaskCompletionEvidenceKind::FocusedVerification,
            execution.call.id.clone(),
            format!("{}-0", execution.call.id),
            status,
        )
    });
    WorkflowV2CallRecord::new(
        "run",
        execution.call.clone(),
        1,
        stable_value_hash(&execution.input),
        result,
        Vec::new(),
    )
    .with_completion_evidence(evidence.into_iter().collect())
}

/// A resume: nothing in `records` was written or replayed by this session.
fn fresh(_: &str) -> bool {
    false
}

fn content_match(candidate: &WorkflowV2CallExecution, record: &WorkflowV2CallRecord) -> bool {
    record.matches_input_for_source_and_scaffold(&stable_value_hash(&candidate.input), None, None)
}

#[test]
fn rebasing_rewrites_whole_ordinal_tokens_only() {
    assert_eq!(
        ordinal_token("verification-wave-review-verify-task-a-1-32"),
        Some(("review-verify-task-a-1", "32"))
    );
    assert_eq!(
        ordinal_token("review-remediate-task-a-1-31"),
        Some(("review-remediate-task-a-1", "31"))
    );
    assert_eq!(ordinal_token("adversarial-review-map"), None);
    let token = "review-verify-task-a-1";
    assert_eq!(
        rebase_text(
            "verification-wave-review-verify-task-a-1-30 and review-verify-task-a-1-30-check",
            token,
            "30",
            "32"
        ),
        "verification-wave-review-verify-task-a-1-32 and review-verify-task-a-1-32-check"
    );
    assert_eq!(
        rebase_text("review-verify-task-a-1-301", token, "30", "32"),
        "review-verify-task-a-1-301",
        "a longer ordinal is a different call"
    );
}

#[test]
fn a_drifted_verifier_replays_the_same_work_recorded_under_another_ordinal() {
    let recorded = verify_execution("TASK-A", 1, 32, "[finding-1]");
    let records = vec![record_for(&recorded, WorkflowV2Status::Accepted)];
    let now = verify_execution("TASK-A", 1, 30, "[finding-1]");
    let replay = remediation_replay_record(&now, &records, fresh, content_match).expect("replayed");
    assert_eq!(replay.call.id, recorded.call.id);
}

#[test]
fn drift_never_crosses_findings_rounds_tasks_or_a_restart() {
    let recorded = verify_execution("TASK-A", 1, 32, "[finding-1]");
    let records = vec![record_for(&recorded, WorkflowV2Status::Accepted)];
    let other_findings = verify_execution("TASK-A", 1, 30, "[finding-2]");
    assert!(remediation_replay_record(&other_findings, &records, fresh, content_match).is_none());
    let other_round = verify_execution("TASK-A", 2, 30, "[finding-1]");
    assert!(remediation_replay_record(&other_round, &records, fresh, content_match).is_none());
    let other_task = verify_execution("TASK-B", 1, 30, "[finding-1]");
    assert!(remediation_replay_record(&other_task, &records, fresh, content_match).is_none());
    let mut restarted = records.clone();
    restarted[0].invalidated_by = Some("restart-stage".to_string());
    let now = verify_execution("TASK-A", 1, 30, "[finding-1]");
    assert!(remediation_replay_record(&now, &restarted, fresh, content_match).is_none());
    let mut plain = now.clone();
    plain.call.options.extra.clear();
    assert!(
        remediation_replay_record(&plain, &records, fresh, content_match).is_none(),
        "only calls carrying a remediation contract drift"
    );
}

#[test]
fn a_superseded_round_is_history_and_a_last_round_is_not() {
    let round_one = verify_execution("TASK-A", 1, 38, "[finding-1]");
    let round_two = verify_execution("TASK-A", 2, 40, "[finding-1]");
    let rejected = record_for(&round_one, WorkflowV2Status::NeedsReview);
    let records = vec![
        rejected.clone(),
        record_for(&round_two, WorkflowV2Status::Accepted),
    ];
    let replay =
        remediation_replay_record(&round_one, &records, fresh, content_match).expect("history");
    assert_eq!(replay.call.id, round_one.call.id);
    assert_eq!(replay.status, WorkflowV2Status::NeedsReview);
    let drifted = verify_execution("TASK-A", 1, 36, "[finding-1]");
    assert_eq!(
        remediation_replay_record(&drifted, &records, fresh, content_match)
            .map(|record| record.call.id.clone()),
        Some(round_one.call.id.clone()),
        "a superseded round under another ordinal is history too"
    );
    let alone = vec![rejected.clone()];
    assert!(
        remediation_replay_record(&round_one, &alone, fresh, content_match).is_none(),
        "the last round of a unit is not history: it may have been in flight"
    );
    let mut restarted = records.clone();
    restarted[1].invalidated_by = Some("restart-stage".to_string());
    assert!(remediation_replay_record(&round_one, &restarted, fresh, content_match).is_none());
}

#[test]
fn a_reusable_record_wins_over_history() {
    let own = verify_execution("TASK-A", 1, 38, "[finding-1]");
    let accepted_sibling = verify_execution("TASK-A", 1, 44, "[finding-1]");
    let round_two = verify_execution("TASK-A", 2, 40, "[finding-1]");
    let records = vec![
        record_for(&own, WorkflowV2Status::NeedsReview),
        record_for(&accepted_sibling, WorkflowV2Status::Accepted),
        record_for(&round_two, WorkflowV2Status::Accepted),
    ];
    let replay = remediation_replay_record(&own, &records, fresh, content_match).expect("replayed");
    assert_eq!(replay.call.id, accepted_sibling.call.id);
}

#[test]
fn a_restarted_call_is_never_answered_by_a_sibling_or_history() {
    let sibling = verify_execution("TASK-A", 1, 32, "[finding-1]");
    let now = verify_execution("TASK-A", 1, 30, "[finding-1]");
    let mut own = record_for(&now, WorkflowV2Status::Accepted);
    own.invalidated_by = Some("restart-stage".to_string());
    let records = vec![record_for(&sibling, WorkflowV2Status::Accepted), own];
    assert!(remediation_replay_record(&now, &records, fresh, content_match).is_none());
}

#[test]
fn inside_one_session_the_same_question_is_asked_again() {
    // Acceptance re-asks an identical fix while its check still fails, and a
    // transport retry re-asks a dead call: both are this session's records.
    let earlier = verify_execution("TASK-A", 1, 32, "[finding-1]");
    let records = vec![record_for(&earlier, WorkflowV2Status::Accepted)];
    let again = verify_execution("TASK-A", 1, 36, "[finding-1]");
    let this_session = |id: &str| id == earlier.call.id;
    assert!(remediation_replay_record(&again, &records, this_session, content_match).is_none());
}

#[test]
fn a_later_round_recorded_before_the_review_last_ran_supersedes_nothing() {
    let round_one = verify_execution("TASK-A", 1, 38, "[finding-1]");
    let round_two = verify_execution("TASK-A", 2, 40, "[finding-1]");
    let mut stale_round_two = record_for(&round_two, WorkflowV2Status::Accepted);
    stale_round_two.started_at = "2026-01-01T00:00:00+00:00".to_string();
    let mut rejected = record_for(&round_one, WorkflowV2Status::NeedsReview);
    rejected.started_at = "2026-01-03T00:00:00+00:00".to_string();
    let mut reduce = record_for(
        &verify_execution("TASK-Z", 1, 1, "[x]"),
        WorkflowV2Status::Accepted,
    );
    reduce.call.id = "adversarial-review-reduce".to_string();
    reduce.started_at = "2026-01-02T00:00:00+00:00".to_string();
    let records = vec![rejected, stale_round_two, reduce];
    assert!(
        remediation_replay_record(&round_one, &records, fresh, content_match).is_none(),
        "the review re-ran after that round 2: it answered other findings"
    );
}

#[test]
fn a_round_the_host_got_no_answer_for_is_not_history() {
    let round_one = verify_execution("TASK-A", 1, 38, "[finding-1]");
    let round_two = verify_execution("TASK-A", 2, 40, "[finding-1]");
    let mut transport = record_for(&round_one, WorkflowV2Status::Failed);
    transport.result.summary = "agent transport failed: connection reset".to_string();
    let records = vec![
        transport,
        record_for(&round_two, WorkflowV2Status::Accepted),
    ];
    assert!(remediation_replay_record(&round_one, &records, fresh, content_match).is_none());
    let mut unanswered = record_for(&round_one, WorkflowV2Status::NeedsReview);
    unanswered.result.data = serde_json::json!({ "outcomes": [] });
    let records = vec![
        unanswered,
        record_for(&round_two, WorkflowV2Status::Accepted),
    ];
    assert!(remediation_replay_record(&round_one, &records, fresh, content_match).is_none());
}
