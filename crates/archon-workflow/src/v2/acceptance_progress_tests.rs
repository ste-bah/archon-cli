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
    let rounds: Vec<AcceptanceRoundRecordV1> = (1..=5)
        .map(|n| {
            let checks = (n..=5)
                .map(|id| failed(&format!("AC-{id}"), "assertion failed"))
                .collect();
            round(n, checks)
        })
        .collect();
    for n in 1..rounds.len() {
        let decision = decide(&rounds[..n], &rounds[n]);
        assert!(!decision.final_round, "round {} shrank the set", n + 1);
        assert!(!decision.escalate);
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

/// Issue 262: no round count ends the loop. It runs past the old ceiling of
/// twelve while it progresses, and only the runaway guard -- a loop that
/// keeps moving to new failing states without clearing -- pauses it.
#[test]
fn no_round_count_ends_the_loop_and_only_the_runaway_guard_pauses_it() {
    let history: Vec<_> = (1..ACCEPTANCE_RUNAWAY_GUARD)
        .map(|n| round(n as u32, vec![failed(&format!("AC-{n}"), "x")]))
        .collect();
    for n in 12..history.len() {
        let decision = decide(&history[..n], &history[n]);
        assert!(!decision.final_round, "round {} progressed", n + 1);
        assert_eq!(decision.pause, None, "round {} progressed", n + 1);
    }
    let next = round(ACCEPTANCE_RUNAWAY_GUARD as u32, vec![failed("AC-NEW", "x")]);
    assert!(made_progress(&history, &next));
    let decision = decide(&history, &next);
    assert!(!decision.final_round);
    assert_eq!(decision.pause, Some(PAUSE_RUNAWAY_GUARD));
    assert!(next.blocks_completion());
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
