//! Issue-121: a HIGH gap a second-pass round's verifier records -- refused
//! or not -- is weighed at the final gate and planned by the bounded third
//! pass, unless the host's own later test runs answer it.

use super::gate_tests::slot;
use super::second_pass_tests::*;
use super::tests::*;
use super::*;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2Status};
use serde_json::json;

const B: &str = "crates/b/src/lib.rs";
const B_TESTS: &str = "cargo test -p b --test registry";
const RED: &str = "registry_roundtrip_keeps_versions";

fn third_slot() -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert(RESIDUAL_GAPS_MARKER.into(), Value::Bool(true));
    options.extra.insert(RESIDUAL_PASS_KEY.into(), json!(3));
    WorkflowV2HostCall {
        id: "residual-gaps-3".into(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    }
}

fn third(w: &World) -> ResidualPlan {
    let records = session_records(&w.store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    third_pass_plan(&refs, &w.store, Some(&w.universe), Some(w.root()))
}

fn keys(plan: &ResidualPlan) -> Vec<String> {
    plan.rounds.iter().map(|round| round.key.clone()).collect()
}

fn pause() {
    std::thread::sleep(std::time::Duration::from_millis(5));
}

/// The live shape: a first-pass round of a medium gap refused over red
/// tests, its second-pass retry, and that retry's verifier REFUSING while
/// it records `gaps` (a regression in another task's file). Returns the
/// executed calls through the second pass and its retry round.
fn refused_second_pass(
    w: &World,
    gaps: &[(&str, &str, &str)],
) -> (Vec<WorkflowV2HostCall>, PlannedRound) {
    let (recorded, first) = first_round(w, "medium");
    let fix1 = round_call(&first, "review-remediate-residual-3", "remediate", 1);
    w.save(&record(
        fix1.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    pause();
    let verify1 = round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &["store::tests::stale"],
        &[],
    );
    w.save(&verify1);
    let retry = second(w).rounds[0].clone();
    pause();
    let fix2 = round_call(&retry, "review-remediate-residual-5", "remediate", 2);
    w.save(&record(
        fix2.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    pause();
    let verify2 = record(
        round_call(
            &retry,
            "verification-wave-review-verify-residual-6",
            "verify",
            2,
        ),
        WorkflowV2Status::NeedsReview,
        &["TASK-A"],
        gaps,
    );
    w.save(&verify2);
    let calls = vec![
        recorded.call.clone(),
        slot(),
        fix1,
        verify1.call.clone(),
        second_slot(),
        fix2,
        verify2.call.clone(),
    ];
    (calls, retry)
}

fn regression() -> (&'static str, &'static str, &'static str) {
    (
        "gap-regression",
        "high",
        "crates/b/src/lib.rs:12 no longer derives the version; registry_roundtrip_keeps_versions is red",
    )
}

/// The host's own base-commit run of `command` for the stage `stage`.
fn host_run(w: &World, stage: &str, tasks: &[&str], command: &str, red: &[&str]) {
    let record: crate::v2::write::test_baseline::BranchBaseline = serde_json::from_value(json!({
        "schema_version": 1, "stage_id": stage, "branch_id": format!("{stage}-0"),
        "base_commit": "c", "canonical_task_ids": tasks,
        "commands": [{"command": command, "base_commit": "c",
            "exit_code": if red.is_empty() { 0 } else { 101 }, "timed_out": false,
            "duration_ms": 1, "failing_tests": red, "cached": false}],
        "obligations": [], "routed": [], "ignored": [], "inherited": [], "pre_existing": []}))
    .unwrap();
    crate::v2::write::test_baseline::save_record(&w.store, &record);
}

#[test]
fn a_refused_second_pass_verifiers_high_gap_blocks_and_gets_one_third_pass_round() {
    let w = package_world();
    let (mut calls, retry) = refused_second_pass(&w, &[regression()]);
    // Before Issue-121 a refused verdict's HIGH gap was never weighed: the
    // only gaps left were medium, and the run went green over the regression.
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert_eq!(gate.blocking.len(), 1, "{gate:#?}");
    assert!(
        gate.blocking[0].contains("gap-regression")
            && gate.blocking[0].contains("after the second residual pass"),
        "{gate:#?}"
    );
    let (first_keys, second_keys) = (keys(&w.plan()), keys(&second(&w)));
    let plan = third(&w);
    assert!(plan.reported.is_empty(), "{:?}", plan.reported);
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.rounds);
    let round = plan.rounds[0].clone();
    // The regression's file is TASK-B's: its owner's round, nothing granted.
    assert_eq!(round.kind, RoundKind::Owned);
    assert_eq!(ids(&round.tasks), ["TASK-B"]);
    assert!(round.files.is_empty());
    assert!(round.key.ends_with("p3") && round.key != retry.key);
    assert_eq!(round.residuals[0].id, "gap-regression");
    calls.push(third_slot());
    let pending = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        pending
            .blocking
            .iter()
            .any(|b| b.contains("gap-regression") && b.contains("did not resolve")),
        "{pending:#?}"
    );
    // The dispatch check answers the third pass's round, marked as one.
    let exec = |pass: u64| crate::v2::WorkflowV2CallExecution {
        call: {
            let mut c = round_call(
                &round,
                "verification-wave-review-verify-residual-8",
                "verify",
                pass,
            );
            c.options.task = Some(round_claim(&round));
            c
        },
        input: json!({"source_data": [{"canonical_task_ids": ["TASK-B"], "target_files": []}]}),
        depends_on: vec![],
    };
    assert_eq!(
        residual_refusal(&exec(3), &w.store, Some(&w.universe), Some(w.root())),
        None
    );
    assert!(
        residual_refusal(&exec(2), &w.store, Some(&w.universe), Some(w.root()))
            .unwrap()
            .contains("no round of the host's plan")
    );
    // Its round runs: the owner fixes the file and its verifier accepts.
    pause();
    let fix3 = round_call(&round, "review-remediate-residual-7", "remediate", 3);
    w.save(&record(
        fix3.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-B"],
        &[],
    ));
    pause();
    let verify3 = round_verdict(
        &round,
        "verification-wave-review-verify-residual-8",
        3,
        &[],
        &[],
    );
    w.save(&verify3);
    calls.extend([fix3, verify3.call.clone()]);
    let green = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(green.blocking.is_empty(), "{green:#?}");
    assert!(
        green
            .notes
            .iter()
            .any(|n| n.contains(&round.key) && n.contains("resolved")),
        "{green:#?}"
    );
    // No pass's plan moved: the first two never read these records, and the
    // third never reads its own rounds'.
    assert_eq!(keys(&w.plan()), first_keys);
    assert_eq!(keys(&second(&w)), second_keys);
    assert_eq!(keys(&third(&w)), [round.key.clone()]);
    // A gap its own verifier records after the third pass has no fourth.
    pause();
    let late = round_verdict(
        &round,
        "verification-wave-review-verify-residual-9",
        3,
        &[],
        &[("gap-late", "high", "crates/a/src/lib.rs lost a field")],
    );
    w.save(&late);
    let after = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        after
            .blocking
            .iter()
            .any(|b| b.contains("gap-late") && b.contains("third and final")),
        "{after:#?}"
    );
    assert_eq!(keys(&third(&w)), [round.key.clone()]);
}

#[test]
fn a_refused_verifiers_high_gap_the_hosts_later_run_answers_is_not_planned_and_does_not_block() {
    let w = package_world();
    let restated = (
        "TASK-A",
        "blocking",
        "Retained verification failure for TASK-A: must-pass tests red",
    );
    let (mut calls, _) = refused_second_pass(&w, &[regression(), restated]);
    host_run(
        &w,
        "verification-wave-review-verify-residual-6",
        &["TASK-A"],
        B_TESTS,
        &[RED],
    );
    // Unanswered: the regression's owner round, and the restated refusal
    // (it names no file) in its own task's round.
    let plan = third(&w);
    assert_eq!(plan.rounds.len(), 2, "{:?}", plan.rounds);
    assert!(
        plan.rounds.iter().all(|r| r.kind == RoundKind::Owned),
        "{:?}",
        plan.rounds
    );
    let tasks: Vec<Vec<&str>> = plan.rounds.iter().map(|r| ids(&r.tasks)).collect();
    assert!(
        tasks.contains(&vec!["TASK-A"]) && tasks.contains(&vec!["TASK-B"]),
        "{tasks:?}"
    );
    // A later accepted verifier whose host run of the same command is still
    // red answers nothing.
    pause();
    let later = verdict(
        "verification-wave-review-verify-task-a-9-10",
        &["TASK-A"],
        &[],
    );
    w.save(&later);
    host_run(&w, &later.call.id, &["TASK-A"], B_TESTS, &[RED]);
    assert_eq!(third(&w).rounds.len(), 2);
    // Passing outright, it answers both: the pass plans nothing, and the
    // gate notes them instead of blocking.
    host_run(&w, &later.call.id, &["TASK-A"], B_TESTS, &[]);
    let plan = third(&w);
    assert!(
        plan.rounds.is_empty() && plan.reported.is_empty(),
        "{plan:?}"
    );
    calls.extend([third_slot(), later.call.clone()]);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(gate.blocking.is_empty(), "{gate:#?}");
    assert_eq!(
        gate.notes
            .iter()
            .filter(|n| n.contains("is answered"))
            .count(),
        2,
        "{gate:#?}"
    );
    // A later host run of the same command red again -- any stage, here one
    // still running -- takes the answer back: the gap is planned and blocks.
    host_run(
        &w,
        "review-remediate-residual-11",
        &["TASK-B"],
        B_TESTS,
        &[RED],
    );
    assert_eq!(third(&w).rounds.len(), 2);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking.iter().any(|b| b.contains("gap-regression")),
        "{gate:#?}"
    );
}

#[test]
fn once_a_third_pass_round_runs_later_evidence_never_moves_the_plan() {
    let w = package_world();
    let restated = (
        "TASK-A",
        "blocking",
        "Retained verification failure for TASK-A: must-pass tests red",
    );
    refused_second_pass(&w, &[regression(), restated]);
    host_run(
        &w,
        "verification-wave-review-verify-residual-6",
        &["TASK-A"],
        B_TESTS,
        &[RED],
    );
    let asked = keys(&third(&w));
    assert_eq!(asked.len(), 2, "{asked:?}");
    // The first round starts; its verifier accepts over a green host run
    // that would answer both gaps.
    pause();
    let round = third(&w).rounds[0].clone();
    let fix = round_call(&round, "review-remediate-residual-7", "remediate", 3);
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    w.save(&record(fix, WorkflowV2Status::Accepted, &tasks, &[]));
    pause();
    let verify = round_verdict(
        &round,
        "verification-wave-review-verify-residual-8",
        3,
        &[],
        &[],
    );
    w.save(&verify);
    host_run(&w, &verify.call.id, &tasks, B_TESTS, &[]);
    // Both rounds stand as asked: the second is still dispatchable.
    assert_eq!(keys(&third(&w)), asked);
}

#[test]
fn the_third_pass_plans_at_most_two_rounds_and_reports_the_rest() {
    let w = package_world();
    let (_, _) = refused_second_pass(
        &w,
        &[
            regression(),
            ("gap-a", "high", "crates/a/src/lib.rs:3 drops the lane"),
            ("gap-pathless", "high", "the lanes disagree"),
        ],
    );
    let plan = third(&w);
    assert_eq!(plan.rounds.len(), MAX_THIRD_PASS_ROUNDS);
    assert!(
        plan.rounds.iter().all(|r| r.kind == RoundKind::Owned),
        "{:?}",
        plan.rounds
    );
    assert_eq!(plan.reported.len(), 1);
    assert_eq!(plan.reported[0].0.id, "gap-pathless");
    assert!(
        plan.reported[0].1.contains("at most 2"),
        "{:?}",
        plan.reported
    );
    // Medium gaps and the second-pass round's own gaps are never planned.
    let w = package_world();
    refused_second_pass(
        &w,
        &[("gap-medium", "medium", B), ("gap-store", "high", STORE)],
    );
    assert!(third(&w).rounds.is_empty(), "{:?}", third(&w).rounds);
}

/// A third-pass round plans a gap a REFUSED verifier recorded, so its prompt
/// must not tell the agent "accepted verifiers" recorded it. The first and
/// second passes' claims keep their wording: they are dispatched call inputs
/// a resumed run replays.
#[test]
fn a_third_pass_prompt_does_not_claim_the_gaps_came_from_accepted_verifiers() {
    let w = package_world();
    let (_, retry) = refused_second_pass(&w, &[regression()]);
    let round = third(&w).rounds[0].clone();
    assert_eq!(round.pass, 3);
    let mut adjudication = round.clone();
    adjudication.kind = RoundKind::Adjudication;
    for claim in [round_claim(&round), round_claim(&adjudication)] {
        assert!(!claim.contains("accepted verifier"), "{claim}");
        assert!(
            claim.contains("whatever") && claim.contains("a refused verifier's HIGH gap counts"),
            "{claim}"
        );
    }
    // Earlier passes: the wording the live run's rounds were dispatched with.
    assert_eq!(retry.pass, 2);
    assert!(
        round_claim(&retry).contains(&format!(
            "Host round {}: accepted verifiers recorded these residual gaps; the host routed",
            retry.key
        )),
        "{}",
        round_claim(&retry)
    );
    let first = w.plan().rounds[0].clone();
    assert_eq!(first.pass, 1);
    let mut first_adjudication = first.clone();
    first_adjudication.kind = RoundKind::Adjudication;
    assert!(round_claim(&first_adjudication).contains(&format!(
        "Read-only ADJUDICATION (host round {}) of residual gap(s) an accepted verifier recorded against",
        first.key
    )));
}
