//! Batch L: where a remediation landing stands, from its unit's records.

use super::plan::{Standing, units};
use super::*;
use crate::v2::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2WriteMode,
};
use crate::write_coordinator::patch_apply::ProjectInputLanding;

const SECOND: i64 = 1_000_000_000;

fn at(second: i64) -> String {
    chrono::DateTime::from_timestamp(1_800_000_000 + second, 0)
        .unwrap()
        .to_rfc3339()
}

fn ns(second: i64) -> i64 {
    (1_800_000_000 + second) * SECOND
}

fn record(
    id: &str,
    stage: &str,
    round: u64,
    status: WorkflowV2Status,
    second: i64,
) -> WorkflowV2CallRecord {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        serde_json::json!({"version": 1, "stage": stage, "taskId": "TASK-A", "round": round,
            "maxRounds": 2, "sourceReduceCallIds": ["reduce"],
            "observedBy": ["acceptance-contract-run-1"]}),
    );
    let fix = stage == "remediate";
    let call = WorkflowV2HostCall {
        id: id.into(),
        method: if fix {
            WorkflowV2HostMethod::Fanout
        } else {
            WorkflowV2HostMethod::Parallel
        },
        write_mode: fix.then_some(WorkflowV2WriteMode::Worktree),
        options,
    };
    let mut result = WorkflowV2Result::accepted("done");
    result.status = status;
    let mut record = WorkflowV2CallRecord::new("run", call, 1, "hash".to_string(), result, vec![]);
    record.status = status;
    record.finished_at = at(second);
    record.started_at = at(second);
    record
}

fn standing(records: &[WorkflowV2CallRecord], landed: i64) -> Standing {
    let units = units(records);
    let unit = units.values().next().expect("one unit");
    unit.standing(super::plan::Place::At(ns(landed)))
}

use WorkflowV2Status::{Accepted, Failed as Rejected, NeedsReview};

#[test]
fn a_landing_its_verifier_did_not_accept_is_refused() {
    let records = [
        record("fix-1", "remediate", 1, Accepted, 10),
        record("verify-1", "verify", 1, NeedsReview, 20),
    ];
    assert!(
        matches!(standing(&records, 9), Standing::Refused { verdict, .. } if verdict == "verify-1")
    );
}

#[test]
fn a_landing_a_later_accepted_verdict_judged_stands() {
    let records = [
        record("fix-1", "remediate", 1, Accepted, 10),
        record("verify-1", "verify", 1, NeedsReview, 20),
        record("fix-2", "remediate", 2, Accepted, 30),
        record("verify-2", "verify", 2, Accepted, 40),
    ];
    assert_eq!(standing(&records, 9), Standing::Stands);
    assert_eq!(standing(&records, 29), Standing::Stands);
    assert_eq!(standing(&records, 41), Standing::Pending);
}

#[test]
fn an_accepted_verdict_over_a_fix_that_was_not_accepted_does_not_cover_it() {
    let records = [
        record("fix-1", "remediate", 1, Rejected, 10),
        record("verify-1", "verify", 1, Accepted, 20),
    ];
    assert!(matches!(standing(&records, 9), Standing::Refused { .. }));
}

#[test]
fn an_interrupted_verdict_judged_nothing() {
    let mut verdict = record("verify-1", "verify", 1, NeedsReview, 20);
    verdict.result.data = serde_json::json!({"interrupted": "paused"});
    let records = [record("fix-1", "remediate", 1, Accepted, 10), verdict];
    assert_eq!(standing(&records, 9), Standing::Pending);
}

#[test]
fn a_no_patch_checkpoint_judged_nothing() {
    let mut checkpoint = record("verify-1-no-patch", "verify", 1, Accepted, 20);
    checkpoint.call.method = WorkflowV2HostMethod::Checkpoint;
    let records = [record("fix-1", "remediate", 1, Accepted, 10), checkpoint];
    assert_eq!(standing(&records, 9), Standing::Pending);
}

#[test]
fn a_unit_the_acceptance_stage_did_not_route_is_out_of_scope() {
    let mut records = [
        record("fix-1", "remediate", 1, Accepted, 10),
        record("verify-1", "verify", 1, NeedsReview, 20),
    ];
    for record in &mut records {
        let contract = record
            .call
            .options
            .extra
            .get_mut("remediationContract")
            .unwrap();
        contract.as_object_mut().unwrap().remove("observedBy");
    }
    assert!(units(&records).is_empty());
}

#[test]
fn a_landing_accepted_once_stands_whatever_a_later_round_answers() {
    let records = [
        record("fix-1", "remediate", 1, Accepted, 10),
        record("verify-1", "verify", 1, Accepted, 20),
        record("fix-2", "remediate", 2, Accepted, 30),
        record("verify-2", "verify", 2, NeedsReview, 40),
    ];
    assert_eq!(standing(&records, 9), Standing::Stands);
    assert!(
        matches!(standing(&records, 29), Standing::Refused { verdict, .. } if verdict == "verify-2")
    );
}

#[test]
fn no_refusal_means_no_revert_and_no_git() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    for record in [
        record("fix-1", "remediate", 1, Accepted, 10),
        record("verify-1", "verify", 1, Accepted, 20),
    ] {
        store.save_call_record(&record).unwrap();
    }
    // No repository at all: nothing is looked up for a unit that accepted.
    let report = revert_refused_landings(&store, Some(&temp.path().join("absent")));
    assert!(
        report.findings.is_empty() && report.decisions.is_empty(),
        "{report:?}"
    );
}

fn line(stage: &str, outcome: &str, after: &str, reason: &str) -> ProjectInputLanding {
    ProjectInputLanding {
        stage_id: stage.into(),
        item_id: format!("{stage}-0"),
        task_ids: vec![],
        path: "data/index.json".into(),
        outcome: outcome.into(),
        before: "a".into(),
        after: after.into(),
        reason: reason.into(),
        at: 1,
        created_dirs: Vec::new(),
    }
}

#[test]
fn a_state_another_landing_put_in_place_is_not_the_refused_landings_to_take_out() {
    // An accepted landing left `b`; the refused one, seeded before it, found
    // `b` already there and logged "already in place".
    let lines = [
        line("accepted", "intent", "b", ""),
        line("accepted", "applied", "b", ""),
        line("refused", "intent", "b", ""),
        line("refused", "applied", "b", "already in place"),
    ];
    assert!(!super::placed_by_itself(&lines, 3));
    // Its own interrupted apply placed the bytes: it is its landing.
    let lines = [
        line("refused", "intent", "b", ""),
        line("refused", "intent", "b", ""),
        line("refused", "applied", "b", "already in place"),
    ];
    assert!(super::placed_by_itself(&lines, 2));
    assert!(super::placed_by_itself(
        &[line("refused", "applied", "b", "")],
        0
    ));
}

#[test]
fn a_fix_record_a_later_session_wrote_again_still_pairs_with_its_rounds_verdict() {
    // The replayed fix's record is newer than the verdict that judged it.
    let records = [
        record("fix-1", "remediate", 1, Accepted, 30),
        record("verify-1", "verify", 1, Accepted, 20),
    ];
    assert_eq!(standing(&records, 9), Standing::Stands);
}

#[test]
fn a_fix_whose_verification_never_completed_is_pending_and_no_candidate() {
    let mut verdict = record("verify-1", "verify", 1, NeedsReview, 20);
    verdict.result.data = serde_json::json!({"interrupted": "paused"});
    let records = [record("fix-1", "remediate", 1, Accepted, 10), verdict];
    let units = units(&records);
    let unit = units.values().next().unwrap();
    assert!(!unit.may_refuse());
    assert_eq!(
        unit.standing(super::plan::Place::At(ns(9))),
        Standing::Pending
    );
}

#[test]
fn a_verify_the_transport_cut_off_or_a_stop_cancelled_judged_nothing() {
    let mut cancelled = record("verify-1", "verify", 1, WorkflowV2Status::Cancelled, 20);
    cancelled.result.summary = "stopped".into();
    let records = [record("fix-1", "remediate", 1, Accepted, 10), cancelled];
    assert_eq!(standing(&records, 9), Standing::Pending);
    let mut cut = record("verify-1", "verify", 1, WorkflowV2Status::Failed, 20);
    cut.result.summary = "agent transport failed: connection reset".into();
    let records = [record("fix-1", "remediate", 1, Accepted, 10), cut];
    assert_eq!(standing(&records, 9), Standing::Pending);
}
