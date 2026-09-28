use super::*;

use crate::v2::result::WorkflowV2Result;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn call(id: &str, contract: Option<Value>) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    if let Some(contract) = contract {
        options
            .extra
            .insert("remediationContract".to_string(), contract);
    }
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: None,
        options,
    }
}

fn acceptance_fix(id: &str) -> WorkflowV2HostCall {
    call(
        id,
        Some(
            serde_json::json!({ "version": 1, "stage": "remediate", "taskId": "TASK-A",
            "round": 1, "maxRounds": 1, "sourceReduceCallIds": ["review-reduce"],
            "observedBy": ["acceptance-contract-run-1"] }),
        ),
    )
}

fn review_fix(id: &str) -> WorkflowV2HostCall {
    call(
        id,
        Some(
            serde_json::json!({ "version": 1, "stage": "remediate", "taskId": "TASK-A",
            "round": 1, "maxRounds": 2, "sourceReduceCallIds": ["review-reduce"] }),
        ),
    )
}

fn recorded(call: WorkflowV2HostCall, at: &str) -> WorkflowV2CallRecord {
    let mut record = WorkflowV2CallRecord::new(
        "run",
        call,
        1,
        "input".to_string(),
        WorkflowV2Result::accepted("answered"),
        Vec::new(),
    );
    record.started_at = at.to_string();
    record.finished_at = at.to_string();
    record
}

fn at(text: &str) -> Option<DateTime<Utc>> {
    parse(text)
}

fn store() -> (tempfile::TempDir, WorkflowV2ResultStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(dir.path().join("v2"));
    (dir, store)
}

const T1: &str = "2026-09-27T22:57:05+00:00";
const T2: &str = "2026-09-28T00:09:14+00:00";
const T3: &str = "2026-09-28T13:16:59+00:00";

#[test]
fn the_question_is_formed_from_the_reduces_and_the_observations_its_contract_names() {
    assert_eq!(
        question_sources(&acceptance_fix("fix-1")),
        vec!["review-reduce", "acceptance-contract-run-1"]
    );
    assert_eq!(
        question_sources(&review_fix("fix-1")),
        vec!["review-reduce"]
    );
    assert!(question_sources(&call("plain-1", None)).is_empty());
}

/// The live wf-0ddadd81 shape: the fix answered the failure acceptance
/// observed at T1; acceptance observed the same failure again at T3, on a
/// tree that held the fix. The recorded answer is not an answer to T3's.
#[test]
fn an_answer_older_than_the_latest_observation_answers_another_question() {
    let records = vec![
        recorded(call("review-reduce", None), "2026-09-24T21:47:56+00:00"),
        recorded(call("acceptance-contract-run-1", None), T3),
    ];
    let (_dir, store) = store();
    let fix = acceptance_fix("review-remediate-task-a-1-81");
    assert!(answer_predates_question(&store, &fix, at(T2), &records));
    assert!(
        !answer_predates_question(&store, &fix, at("2026-09-28T13:20:00+00:00"), &records),
        "an answer given after the observation answers it"
    );
    assert!(
        answer_predates_question(&store, &fix, None, &records),
        "an answer of unknown age is no answer"
    );
    assert!(
        answer_predates_question(&store, &fix, at(T3), &records[..1]),
        "a named observation with no record is taken as observed now"
    );
}

/// Review remediation on every resume: the reduces are replayed, never
/// re-recorded, so every answer given after them still answers.
#[test]
fn review_remediation_answered_after_its_reduces_replays() {
    let records = vec![recorded(
        call("review-reduce", None),
        "2026-09-24T21:47:56+00:00",
    )];
    let (_dir, store) = store();
    let fix = review_fix("review-remediate-task-a-1-47");
    assert!(!answer_predates_question(&store, &fix, at(T1), &records));
    assert!(
        !answer_predates_question(&store, &call("plain-1", None), None, &records),
        "no contract, nothing to judge"
    );
    assert!(
        !answer_predates_question(&store, &review_fix("x-1"), None, &[]),
        "no recorded source constrains nothing"
    );
}

/// A re-save (a replay's new attempt) is dated when it was written; the
/// execution it restates is older, and that is what is judged.
#[test]
fn a_recorded_answer_is_judged_by_the_execution_it_restates() {
    let store = WorkflowV2ResultStore::new(tempfile::tempdir().expect("tempdir").path().join("v2"));
    let mut resaved = recorded(acceptance_fix("review-remediate-task-a-1-87"), T3);
    resaved.answered_by = Some(crate::v2::result_store::WorkflowV2AnswerOrigin {
        call_id: "review-remediate-task-a-1-83".to_string(),
        finished_at: T2.to_string(),
    });
    assert_eq!(record_answered_at(&store, &resaved), at(T2));
    let records = vec![recorded(call("acceptance-contract-run-1", None), T3)];
    assert!(record_predates_question(
        &store,
        &acceptance_fix("review-remediate-task-a-1-81"),
        &resaved,
        &records
    ));
}

/// A branch answer is as old as the oldest trace of it: its record and its
/// outcome file.
#[test]
fn a_branch_answer_is_as_old_as_its_oldest_trace() {
    let (_dir, store) = store();
    let (call_id, item_id) = (
        "review-remediate-task-a-1-83",
        "review-remediate-task-a-1-83-0",
    );
    assert_eq!(branch_answered_at(&store, call_id, item_id, &[]), None);
    let outcome = store.branch_outcome_path(call_id, item_id);
    std::fs::create_dir_all(outcome.parent().unwrap()).unwrap();
    std::fs::write(&outcome, "{}").unwrap();
    let written = branch_answered_at(&store, call_id, item_id, &[]).expect("outcome file");
    let records = vec![recorded(acceptance_fix(call_id), T2)];
    assert_eq!(
        branch_answered_at(&store, call_id, item_id, &records),
        at(T2).min(Some(written)),
        "the record is older than the file written just now"
    );
}

fn review_round(id: &str, stage: &str, round: u64) -> WorkflowV2HostCall {
    call(
        id,
        Some(
            serde_json::json!({ "version": 1, "stage": stage, "taskId": "TASK-A",
            "round": round, "maxRounds": 2, "sourceReduceCallIds": ["review-reduce"] }),
        ),
    )
}

/// A later round is asked because the earlier round's verdict refused. When
/// this session dispatched the earlier round's fix again, that refusal is
/// new: no earlier session's round-2 answer saw it. A replayed earlier round
/// leaves the recorded answer standing.
#[test]
fn a_later_round_after_an_earlier_round_ran_again_is_a_new_question() {
    let (_dir, store) = store();
    let records = vec![recorded(
        call("review-reduce", None),
        "2026-09-24T21:47:56+00:00",
    )];
    let round_one = review_round("review-remediate-task-a-1-31", "remediate", 1);
    let round_two = review_round("review-remediate-task-a-2-33", "remediate", 2);
    let verdict_two = review_round("verification-wave-review-verify-task-a-2-34", "verify", 2);
    assert!(!answer_predates_question(
        &store,
        &round_two,
        at(T2),
        &records
    ));
    let other_task = call(
        "review-remediate-task-b-1-40",
        Some(
            serde_json::json!({ "version": 1, "stage": "remediate", "taskId": "TASK-B",
            "round": 1, "maxRounds": 2, "sourceReduceCallIds": ["review-reduce"] }),
        ),
    );
    let key = |call: &WorkflowV2HostCall| {
        super::super::resume_verdict::remediation_round_key(call).unwrap()
    };
    store.note_fix_dispatched(&key(&other_task));
    assert!(
        !answer_predates_question(&store, &round_two, at(T2), &records),
        "another unit ran"
    );
    store.note_fix_dispatched(&key(&round_one));
    assert!(answer_predates_question(
        &store,
        &round_two,
        at(T2),
        &records
    ));
    assert!(answer_predates_question(
        &store,
        &verdict_two,
        at(T2),
        &records
    ));
    assert!(
        !answer_predates_question(&store, &round_one, at(T2), &records),
        "round 1 is formed from the reduces alone"
    );
}

/// The hash-free waiver for a completed task's record never covers another
/// question: other findings, targets or contract.
#[test]
fn the_same_question_is_the_same_prompt_targets_and_contract() {
    let mut asked = acceptance_fix("verification-wave-review-verify-task-a-1-82");
    asked.options.task = Some("Findings (verbatim): [f1]".to_string());
    let recorded = asked.clone();
    assert!(asks_the_same(&recorded, &asked));
    let mut other = asked.clone();
    other.options.task = Some("Findings (verbatim): [f2]".to_string());
    assert!(!asks_the_same(&recorded, &other));
    let mut widened = asked.clone();
    widened.options.target_files = vec!["other.rs".to_string()];
    assert!(!asks_the_same(&recorded, &widened));
    assert!(!asks_the_same(&recorded, &review_fix(&asked.id)));
}
