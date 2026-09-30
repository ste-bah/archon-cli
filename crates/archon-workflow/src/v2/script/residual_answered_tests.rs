//! "Answered" (`residual_superseded`) needs every owed red test named
//! passed by a host run recorded since passed ids were kept; a run recorded
//! before is read as it always was, everywhere alike.

use super::super::second_pass_tests::*;
use super::super::tests::*;
use super::super::third_pass_tests::*;
use super::super::*;
use serde_json::json;

/// A red test of ANOTHER command in the recorder's host runs, named by no
/// gap, is still red on the tree the gap was recorded against: a later run
/// that passes only the named test's command answers nothing.
#[test]
fn an_unnamed_red_test_of_another_command_keeps_a_gap_unanswered() {
    let w = package_world();
    let (mut calls, _) = refused_second_pass(&w, &[regression()]);
    const OTHER: &str = "cargo test -p b --lib";
    host_runs(
        &w,
        "verification-wave-review-verify-residual-6",
        &["TASK-A"],
        &[
            (B_TESTS, &[RED], &[]),
            (OTHER, &["lane::tests::drift"], &[]),
        ],
    );
    pause();
    let later = verdict(
        "verification-wave-review-verify-task-a-9-10",
        &["TASK-A"],
        &[],
    );
    w.save(&later);
    host_runs(&w, &later.call.id, &["TASK-A"], &[(B_TESTS, &[], &[RED])]);
    assert_eq!(fresh(&w).len(), 1, "{:?}", third(&w).rounds);
    // Both commands green by id: answered.
    host_runs(
        &w,
        &later.call.id,
        &["TASK-A"],
        &[
            (B_TESTS, &[], &[RED]),
            (OTHER, &[], &["lane::tests::drift"]),
        ],
    );
    assert!(fresh(&w).is_empty(), "{:?}", third(&w).rounds);
    calls.extend([third_slot(), later.call.clone()]);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    // Only the medium gap the refused retry left (planned again, unrun)
    // blocks: Batch O.
    assert!(
        gate.blocking
            .iter()
            .all(|b| b.contains("`gap-store`") || b.contains(REFUSED_RED_GAP_ID)),
        "{gate:#?}"
    );
}

/// A host run recorded before passed ids were kept is read as it always was
/// (passed outright), the same at the slot and after its rounds start, so an
/// upgraded host never moves the pass's plan.
#[test]
fn a_legacy_host_run_answers_as_it_always_did_and_the_plan_holds() {
    let w = package_world();
    let (_, _) = refused_second_pass(&w, &[regression()]);
    pause();
    let later = verdict(
        "verification-wave-review-verify-task-a-9-10",
        &["TASK-A"],
        &[],
    );
    w.save(&later);
    // Legacy: exit 0, no ids, no `passed_ids_kept`.
    let record: crate::v2::write::test_baseline::BranchBaseline = serde_json::from_value(json!({
        "schema_version": 1, "stage_id": later.call.id, "branch_id": format!("{}-0", later.call.id),
        "base_commit": "c", "canonical_task_ids": ["TASK-A"],
        "commands": [{"command": B_TESTS, "base_commit": "c", "exit_code": 0, "timed_out": false,
            "duration_ms": 1, "failing_tests": [], "cached": false}],
        "obligations": [], "routed": [], "ignored": [], "inherited": [], "pre_existing": []}))
    .unwrap();
    // The recorder's own red run, as it was recorded.
    host_run(
        &w,
        "verification-wave-review-verify-residual-6",
        &["TASK-A"],
        B_TESTS,
        &[RED],
    );
    crate::v2::write::test_baseline::save_record(&w.store, &record);
    assert!(fresh(&w).is_empty(), "{:?}", third(&w).rounds);
}

/// The third pass's rounds other than a refused round planned again whole
/// (Batch O), which no host run answers.
fn fresh(w: &World) -> Vec<PlannedRound> {
    third(w)
        .rounds
        .into_iter()
        .filter(|r| r.refusal.is_none())
        .collect()
}
