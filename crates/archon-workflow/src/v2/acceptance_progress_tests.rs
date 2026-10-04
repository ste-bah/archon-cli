//! The acceptance loop follows progress, never a round count (A2).

use super::super::{
    ACCEPTANCE_MAX_ROUNDS, ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION, AcceptanceCheckRecordV1,
    AcceptanceCheckStatus, write_round_record,
};
use super::*;

fn check(id: &str, status: AcceptanceCheckStatus, stderr: &str) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: id.into(),
        criterion: format!("criterion {id}"),
        kind: "command".into(),
        status,
        exit_code: Some(i32::from(status != AcceptanceCheckStatus::Passed)),
        operational_error: None,
        owning_tasks: vec!["TASK-A".into()],
        stdout_tail: String::new(),
        stderr_tail: stderr.into(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    }
}

fn round(n: u32, checks: Vec<AcceptanceCheckRecordV1>) -> AcceptanceRoundRecordV1 {
    AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: "wf-progress".into(),
        call_id: format!("acceptance-contract-run-{n}"),
        round: n,
        attempt: 1,
        max_rounds: ACCEPTANCE_MAX_ROUNDS,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: false,
    }
}

fn failed(id: &str, why: &str) -> AcceptanceCheckRecordV1 {
    check(id, AcceptanceCheckStatus::Failed, why)
}

#[test]
fn the_loop_runs_past_the_old_round_count_while_the_failing_set_shrinks() {
    let mut rounds = vec![failing(0, 1)];
    rounds.extend(
        (1..=5)
            .map(|n| {
                let checks = (n..=5)
                    .map(|id| failed(&format!("AC-{id}"), "assertion failed"))
                    .collect();
                round(n, checks)
            })
            .collect::<Vec<AcceptanceRoundRecordV1>>(),
    );
    for n in 1..rounds.len() {
        let decision = decide(&rounds[..n], &rounds[n]);
        assert!(!decision.final_round, "round {} shrank the set", n + 1);
        assert!(!decision.escalate);
        assert_eq!(decision.stalled_rounds, 0);
        assert_eq!(decision.pause, None);
    }
}

#[test]
fn a_stalled_round_escalates_and_a_second_one_pauses_the_run() {
    let same = || round(1, vec![failed("AC-1", "assertion failed: took 12ms")]);
    let first = same();
    let mut second = same();
    // Digits and scratch paths are not new evidence.
    second.checks[0].stderr_tail = "assertion failed: took 97ms".into();
    let decision = decide(std::slice::from_ref(&first), &second);
    assert!(!decision.final_round);
    assert!(decision.escalate);
    assert_eq!(decision.stalled_rounds, 1);
    let third = same();
    let decision = decide(&[first.clone(), second.clone()], &third);
    // Issue 262: a stall pauses with evidence; it never ends the loop.
    assert!(!decision.final_round);
    assert_eq!(decision.pause, Some(PAUSE_NO_PROGRESS));
    assert_eq!(decision.stalled_rounds, 2);
    assert!(third.blocks_completion());
}

#[test]
fn only_a_new_failing_set_is_progress_never_changed_output_text() {
    let a = round(1, vec![failed("AC-1", "missing field x")]);
    // The same check failing with other words is no progress (Batch O
    // review: output text that differs every round funded the loop for ever).
    let b = round(2, vec![failed("AC-1", "missing field y")]);
    assert!(!made_progress(std::slice::from_ref(&a), &b));
    // A different failing check is a state never seen: progress.
    let c = round(3, vec![failed("AC-2", "missing field x")]);
    assert!(made_progress(&[a.clone(), b.clone()], &c));
    // Back to the first state: an oscillation, not progress.
    let d = round(4, vec![failed("AC-1", "missing field x")]);
    assert!(!made_progress(&[a, b, c], &d));
}

fn failing(n: u32, count: u32) -> AcceptanceRoundRecordV1 {
    round(
        n,
        (0..count)
            .map(|id| failed(&format!("AC-{id}"), "x"))
            .collect(),
    )
}

/// Issue 262: no round count ends or pauses the loop. Every round here
/// reaches a failing set never seen, so round 64 and past never pause.
#[test]
fn a_loop_that_keeps_reaching_new_failing_sets_never_pauses() {
    let mut history = vec![failing(0, 1)];
    history.extend((1..=74).map(|n| failing(n, 200 - n)));
    for n in 1..history.len() {
        let decision = decide(&history[..n], &history[n]);
        assert_eq!(decision.pause, None, "round {} reached a new set", n + 1);
        assert!(!decision.final_round);
        assert_eq!(decision.stalled_rounds, 0);
    }
}

/// Round 3 (decision A): progress is a failing set this run never reached,
/// never beating the all-time minimum. After a regression from 1 failing
/// check to 100, every round that repairs one more reaches a new set: no
/// pause, at round 65 or ever.
#[test]
fn recovery_after_a_regression_is_progress_on_every_new_failing_set() {
    let mut history = vec![round(1, vec![failed("AC-ONLY", "x")])];
    for (n, count) in (2..).zip((37..=100).rev()) {
        let next = failing(n, count);
        let decision = decide(&history, &next);
        assert_eq!(decision.pause, None, "round {n} repaired a check");
        assert_eq!(decision.stalled_rounds, 0, "round {n}");
        history.push(next);
    }
}

/// A failing set reached before counts toward the stall, whatever its size;
/// a set never reached resets the count.
#[test]
fn a_revisited_failing_set_counts_toward_the_stall_and_a_new_one_resets_it() {
    let set = |n: u32, ids: &[&str]| round(n, ids.iter().map(|id| failed(id, "x")).collect());
    let a = set(1, &["AC-1"]);
    let b = set(2, &["AC-2"]);
    let again_a = set(3, &["AC-1"]);
    let c = set(4, &["AC-3"]);
    let history = vec![a.clone(), b.clone()];
    let decision = decide(&history, &again_a);
    assert_eq!((decision.stalled_rounds, decision.escalate), (1, true));
    let history = vec![a.clone(), b.clone(), again_a.clone()];
    assert_eq!(decide(&history, &c).stalled_rounds, 0, "a new set resets");
    let history = vec![a.clone(), b.clone(), again_a.clone(), c.clone()];
    assert_eq!(decide(&history, &set(5, &["AC-1"])).stalled_rounds, 1);
    let history = vec![a, b, again_a, c, set(5, &["AC-1"])];
    let decision = decide(&history, &set(6, &["AC-2"]));
    assert_eq!(decision.stalled_rounds, 2);
    assert_eq!(decision.pause, Some(PAUSE_NO_PROGRESS));
}

#[test]
fn a_clean_round_is_final_and_host_repairable_errors_keep_the_loop_going() {
    let clean = round(1, vec![check("AC-1", AcceptanceCheckStatus::Passed, "")]);
    assert!(decide(&[], &clean).final_round);
    // An erroring check is the host's to repair: never final on round 1.
    let mut errored = check("AC-1", AcceptanceCheckStatus::Error, "");
    errored.owning_tasks.clear();
    errored.operational_error = Some("native observation guardian failed".into());
    let errored = round(1, vec![errored]);
    let decision = decide(&[], &errored);
    assert!(!decision.final_round);
    // So is a round-level error.
    let mut site = round(1, Vec::new());
    site.operational_errors.push("scratch site failed".into());
    assert!(!decide(&[], &site).final_round);
}

#[test]
fn nothing_passes_without_a_contract_or_with_no_check_run() {
    let mut absent = round(1, Vec::new());
    absent.contract_present = false;
    assert!(absent.blocks_completion());
    let empty = round(1, Vec::new());
    assert!(empty.blocks_completion());
}

#[test]
fn earlier_rounds_read_the_latest_attempt_of_each_round() {
    let dir = tempfile::tempdir().unwrap();
    let mut one = round(1, vec![failed("AC-1", "x")]);
    write_round_record(dir.path(), &one).unwrap();
    one.attempt = 2;
    one.checks.push(failed("AC-2", "y"));
    write_round_record(dir.path(), &one).unwrap();
    write_round_record(dir.path(), &round(2, Vec::new())).unwrap();
    let earlier = earlier_rounds(dir.path(), 3);
    assert_eq!(earlier.len(), 2);
    assert_eq!(earlier[0].attempt, 2);
    assert_eq!(earlier[0].checks.len(), 2);
    assert!(earlier_rounds(dir.path(), 1).is_empty());
}

/// Round 3 (decision A): the states reached and the revisit count are
/// persisted, so a resumed round keeps them; a run with no ledger yet gets
/// the one its round records leave.
#[test]
fn the_ledger_is_persisted_and_rebuilt_from_records_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    let a = round(1, vec![failed("AC-1", "x")]);
    let b = round(2, vec![failed("AC-2", "x")]);
    write_round_record(dir.path(), &a).unwrap();
    write_round_record(dir.path(), &b).unwrap();
    let rebuilt = ProgressLedger::load(dir.path(), 3);
    assert_eq!(
        rebuilt,
        ProgressLedger::from_history(&[a.clone(), b.clone()])
    );
    let mut ledger = rebuilt;
    assert_eq!(ledger.observe(&round(3, vec![failed("AC-1", "y")])), 1);
    write_round_record(dir.path(), &round(3, vec![failed("AC-1", "y")])).unwrap();
    ledger.save(dir.path()).unwrap();
    let resumed = ProgressLedger::load(dir.path(), 3);
    assert_eq!(resumed.revisits, 1);
    assert_eq!(resumed.seen.len(), 2);
}

#[test]
fn rebuild_keeps_all_attempts_including_the_resumed_round_and_records_win() {
    let dir = tempfile::tempdir().unwrap();
    let a = round(1, vec![failed("A", "x")]);
    let mut b = round(1, vec![failed("B", "x")]);
    b.attempt = 2;
    let c = round(2, vec![failed("A", "x")]);
    for record in [&a, &b, &c] {
        write_round_record(dir.path(), record).unwrap();
    }
    let expected = ProgressLedger::from_history(&[a.clone(), b, c.clone()]);
    assert_eq!(ProgressLedger::load(dir.path(), 2), expected);
    ProgressLedger::from_history(&[a]).save(dir.path()).unwrap();
    let mut ledger = ProgressLedger::load(dir.path(), 2);
    assert_eq!(
        ledger, expected,
        "a readable stale ledger cannot hide records"
    );
    assert_eq!(decide_with(&mut ledger, &c).pause, Some(PAUSE_NO_PROGRESS));
}
