//! Batch O: nothing a verifier recorded is dropped by a label, a cap or a
//! loose identity -- every gap is planned, and whatever stands blocks.

use super::gate_tests::slot;
use super::second_pass_tests::*;
use super::tests::*;
use super::third_pass_tests::{
    B_TESTS, RED, host_run, keys, pause, refused_second_pass, regression, third,
};
use super::*;
use crate::v2::{WorkflowV2HostCall, WorkflowV2Status};

/// Batch O: a pattern names EVERY file it matches, however many (a bound
/// used to make a wide one name nothing, so its gap lost its routing and was
/// only adjudicated); the gap is planned as a file round granted them all.
#[test]
fn a_high_gap_whose_only_pattern_is_wide_names_every_match_and_is_planned() {
    let w = world();
    for n in 0..=24 {
        std::fs::write(w.root().join(format!("crates/shared/src/f{n}.rs")), "//\n").unwrap();
    }
    w.save(&verdict(
        "verification-wave-review-verify-cross-1-2",
        &["TASK-A", "TASK-B"],
        &[(
            "gap-wide",
            "high",
            "every crates/shared/src/*.rs lane is wrong",
        )],
    ));
    let plan = w.plan();
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.rounds);
    assert!(plan.reported.is_empty(), "{:?}", plan.reported);
    let round = &plan.rounds[0];
    assert_eq!(round.kind, RoundKind::Expansion);
    // f0..f24 and the store itself.
    assert_eq!(
        round.residuals[0].files.len(),
        26,
        "{:?}",
        round.residuals[0].files
    );
    assert_eq!(round.files.len(), 26, "every match is granted");
    assert_eq!(ids(&round.tasks), ["TASK-A", "TASK-B"]);
    let view = crate::v2::script::residual_plan::round_view(round, &w.store);
    assert_eq!(view["dispatchable"], true);
}

/// Batch O: the round's own gap recorded again in OTHER words is not
/// provably the same gap, so it is planned as a new one: an ambiguous match
/// never leaves a gap unplanned.
#[test]
fn a_rounds_gap_recorded_again_in_other_words_is_planned_as_new() {
    let w = package_world();
    let (_, first) = first_round(&w, "medium");
    w.save(&round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[],
        &[(
            "gap-store",
            "high",
            &format!("{STORE}:9 still reads the timeframe from the wrong segment"),
        )],
    ));
    let plan = second(&w);
    let carried: Vec<(&str, &str)> = plan
        .rounds
        .iter()
        .flat_map(|r| {
            r.residuals
                .iter()
                .map(|g| (g.id.as_str(), g.recorded_by.as_str()))
        })
        .collect();
    assert_eq!(
        carried,
        [("gap-store", "verification-wave-review-verify-residual-4")]
    );
}

#[test]
fn identity_is_the_id_the_owning_tasks_and_the_whole_text() {
    let gap = |id: &str, tasks: &[&str], text: &str| Residual {
        recorded_by: "v".into(),
        id: id.into(),
        severity: ResidualSeverity::High,
        description: text.into(),
        files: Vec::new(),
        unit_tasks: tasks.iter().map(|t| t.to_string()).collect(),
        recorded_summary: String::new(),
        host_built: false,
    };
    let text = "crates/shared/src/store.rs:9 reads the timeframe from the wrong segment";
    let original = gap("gap-1", &["TASK-A"], text);
    let owners = original.unit_tasks.clone();
    let same = |candidate: &Residual| dispositions::same_identity(&original, &owners, candidate);
    assert!(same(&gap("GAP-1 ", &["TASK-A"], &format!("  {text}\n"))));
    // A shared id with other words, another unit, or no id: a NEW gap.
    assert!(!same(&gap(
        "gap-1",
        &["TASK-A"],
        &format!("{text}; and the ingest lane")
    )));
    assert!(!same(&gap("gap-1", &["TASK-B"], text)));
    assert!(!same(&gap("gap-2", &["TASK-A"], text)));
    let unnamed = gap("", &["TASK-A"], text);
    assert!(!dispositions::same_identity(&unnamed, &owners, &unnamed));
    // The loose match, read only where it keeps a gap open, still matches.
    assert!(dispositions::same_gap(
        &original,
        "gap-1",
        "other words entirely"
    ));
}

/// A refused verifier's MEDIUM gap recorded after the slot blocks: every
/// standing gap does, whoever recorded it (it used to be skipped).
#[test]
fn a_refused_verifiers_medium_gap_after_the_slot_blocks() {
    let w = world();
    let mut late = verdict(
        "verification-wave-review-verify-task-a-1-20",
        &["TASK-A"],
        &[("gap-late-medium", "low", STORE)],
    );
    late.status = WorkflowV2Status::NeedsReview;
    w.save(&late);
    let calls = vec![slot(), late.call.clone()];
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert_eq!(gate.blocking.len(), 1, "{gate:#?}");
    assert!(
        gate.blocking[0].contains("gap-late-medium") && gate.blocking[0].contains("(medium,"),
        "{gate:#?}"
    );
}

/// Batch O: the third pass plans EVERY round -- file rounds, an
/// adjudication for a gap naming no file, and the refused retry planned
/// again -- at every severity; no cap reports any.
#[test]
fn the_third_pass_plans_every_round_at_every_severity() {
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
    let kinds: Vec<(&str, bool)> = plan
        .rounds
        .iter()
        .map(|r| (r.kind.as_str(), r.refusal.is_some()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("owned", false),
            ("owned", false),
            ("adjudication", false),
            ("expansion", true)
        ],
        "{:?}",
        plan.rounds
    );
    assert!(plan.reported.is_empty(), "{:?}", plan.reported);
    // A medium gap is planned, and the round's own gap recorded again in
    // other words is not provably the same gap: it is planned as new.
    let w = package_world();
    refused_second_pass(
        &w,
        &[
            ("gap-medium", "medium", "crates/b/src/lib.rs"),
            ("gap-store", "high", STORE),
        ],
    );
    let carried: Vec<(String, String)> = third(&w)
        .rounds
        .iter()
        .filter(|r| r.refusal.is_none())
        .flat_map(|r| {
            r.residuals
                .iter()
                .map(|g| (g.id.clone(), g.recorded_by.clone()))
        })
        .collect();
    let by = "verification-wave-review-verify-residual-6".to_string();
    assert!(
        carried.contains(&("gap-medium".into(), by.clone())),
        "{carried:?}"
    );
    assert!(carried.contains(&("gap-store".into(), by)), "{carried:?}");
}

/// Issue-121 (moved here to keep its file bounded): once a third-pass
/// round runs, later evidence never moves the plan.
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
    // Its two rounds and the refused retry planned again (Batch O).
    assert_eq!(asked.len(), 3, "{asked:?}");
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

/// Batch O: on a REFUSED verdict the host's own `review` bookkeeping gap
/// states why it refused (the refusal is what is weighed and planned
/// again), while an agent's low-labelled gap is work like any other; on an
/// ACCEPTED verdict both are work.
#[test]
fn a_refused_verdicts_host_review_gap_is_its_refusal_and_a_low_gap_is_work() {
    let gaps = [
        ("failed_test_command_verification", "review", STORE),
        ("gap-nit", "low", STORE),
    ];
    let mut refused = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &gaps,
    );
    refused.status = WorkflowV2Status::NeedsReview;
    let ids = |record: &WorkflowV2CallRecord| -> Vec<(String, ResidualSeverity)> {
        residuals_of(record, None)
            .into_iter()
            .map(|r| (r.id, r.severity))
            .collect()
    };
    assert_eq!(
        ids(&refused),
        [("gap-nit".into(), ResidualSeverity::Medium)]
    );
    let accepted = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &gaps,
    );
    assert_eq!(ids(&accepted).len(), 2);
}

/// Batch O: a first-pass round its verifier refused is planned again by the
/// second pass; when that retry resolves the gap, the refusing verifier's
/// word-for-word re-record of it is the resolved gap, and nothing blocks.
#[test]
fn a_refused_round_retried_by_the_next_pass_resolves_its_gap() {
    let w = package_world();
    let (recorded, first) = first_round(&w, "medium");
    let fix1 = round_call(&first, "review-remediate-residual-3", "remediate", 1);
    w.save(&record(
        fix1.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    pause();
    let again_text = format!("{STORE}:9 reads the timeframe");
    let verify1 = record(
        round_call(
            &first,
            "verification-wave-review-verify-residual-4",
            "verify",
            1,
        ),
        WorkflowV2Status::NeedsReview,
        &["TASK-A"],
        &[("gap-store", "medium", again_text.as_str())],
    );
    w.save(&verify1);
    let retry = second(&w);
    assert_eq!(retry.rounds.len(), 1, "{:?}", retry.rounds);
    let retry = retry.rounds[0].clone();
    assert_eq!(
        (retry.pass, retry.residuals.clone()),
        (2, first.residuals.clone())
    );
    let calls_before = vec![recorded.call.clone(), slot(), fix1, verify1.call.clone()];
    let gate = |calls: &[WorkflowV2HostCall]| {
        residual_verdict(calls, &w.store, Some(&w.universe), Some(w.root()))
    };
    let mut calls = calls_before.clone();
    calls.push(second_slot());
    assert!(
        gate(&calls)
            .blocking
            .iter()
            .any(|b| b.contains("`gap-store`")),
        "unrun, the retry leaves it standing"
    );
    pause();
    let fix2 = round_call(&retry, "review-remediate-residual-5", "remediate", 2);
    w.save(&record(
        fix2.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    pause();
    let verify2 = round_verdict(
        &retry,
        "verification-wave-review-verify-residual-6",
        2,
        &[],
        &[],
    );
    w.save(&verify2);
    calls.extend([fix2, verify2.call.clone()]);
    let green = gate(&calls);
    assert!(green.blocking.is_empty(), "{green:#?}");
    assert!(
        green.notes.iter().any(|n| n.contains("word for word")),
        "{green:#?}"
    );
}
