//! Issue-117: the final gate over residual gaps, the plan's view, and the
//! review round a refusal over an unowned blocker buys.

use super::tests::*;
use super::*;
use crate::v2::WorkflowV2Result;
use crate::v2::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2HostOptions,
    WorkflowV2Status,
};
use serde_json::json;

fn slot() -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert(RESIDUAL_GAPS_MARKER.into(), Value::Bool(true));
    WorkflowV2HostCall {
        id: "residual-gaps-1".into(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    }
}

fn round_calls(
    w: &World,
    round: &PlannedRound,
    verify: WorkflowV2Status,
) -> Vec<WorkflowV2HostCall> {
    round_calls_recording(w, round, verify, &[])
}

fn round_calls_recording(
    w: &World,
    round: &PlannedRound,
    verify: WorkflowV2Status,
    gaps: &[(&str, &str, &str)],
) -> Vec<WorkflowV2HostCall> {
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let fix = execution(round, true, &[]).call;
    let mut check = execution(round, false, &[]).call;
    check.id = "verification-wave-review-verify-residual-8".into();
    w.save(&record(
        fix.clone(),
        WorkflowV2Status::Accepted,
        &tasks,
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    w.save(&record(check.clone(), verify, &tasks, gaps));
    vec![fix, check]
}

#[test]
fn the_gate_resolves_a_planned_gap_only_on_its_rounds_accepted_records() {
    for (status, resolved) in [
        (WorkflowV2Status::Accepted, true),
        (WorkflowV2Status::NeedsReview, false),
    ] {
        let w = world();
        let recorded = verdict(
            "verification-wave-review-verify-task-a-1-2",
            &["TASK-A"],
            &[
                ("gap-store", "high", STORE),
                ("gap-prose", "medium", "unmapped prose"),
            ],
        );
        w.save(&recorded);
        let round = w.plan().rounds[0].clone();
        let mut calls = vec![recorded.call.clone(), slot()];
        calls.extend(round_calls(&w, &round, status));
        let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
        assert_eq!(gate.blocking.is_empty(), resolved, "{gate:#?}");
        assert!(
            gate.notes
                .iter()
                .any(|n| n.starts_with("warning:") && n.contains("gap-prose")),
            "a medium gap no round carries is a warning: {gate:#?}"
        );
        if !resolved {
            assert!(gate.blocking[0].contains("gap-store"), "{gate:#?}");
            assert!(gate.blocking[0].contains("did not resolve it"), "{gate:#?}");
        }
    }
}

#[test]
fn a_high_gap_recorded_after_the_slot_or_without_one_blocks() {
    let w = world();
    let late = verdict(
        "verification-wave-review-verify-task-a-1-20",
        &["TASK-A"],
        &[("gap-late", "high", STORE)],
    );
    w.save(&late);
    for calls in [vec![slot(), late.call.clone()], vec![late.call.clone()]] {
        let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
        assert_eq!(gate.blocking.len(), 1, "{gate:#?}");
        assert!(gate.blocking[0].contains("gap-late"));
    }
}

#[test]
fn a_refusal_over_an_unowned_blocker_plans_a_review_round_that_discharges_its_unit() {
    let w = world();
    let mut refused = verdict(
        "verification-wave-review-verify-task-b-1-2",
        &["TASK-B"],
        &[],
    );
    refused.status = WorkflowV2Status::NeedsReview;
    let mut blocker = WorkflowV2Evidence::new(WorkflowV2EvidenceKind::Blocker, "the lane is wrong");
    blocker.source = Some(format!("{STORE}:12"));
    refused.result.evidence.push(blocker);
    w.save(&refused);
    let plan = w.plan();
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.rounds);
    let round = plan.rounds[0].clone();
    assert_eq!(round.kind, RoundKind::Review);
    assert_eq!(
        ids(&round.tasks),
        ["TASK-A", "TASK-B"],
        "the unit and the naming task"
    );
    assert_eq!(round.unit_key.as_deref(), Some("TASK-B"));
    let mut calls = vec![refused.call.clone(), slot()];
    calls.extend(round_calls(&w, &round, WorkflowV2Status::Accepted));
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert_eq!(gate.discharged, BTreeSet::from(["TASK-B".to_string()]));
    assert!(gate.blocking.is_empty(), "{gate:#?}");
}

#[test]
fn the_plan_view_rides_only_on_the_checkpoint_that_asked_and_marks_attempted_rounds() {
    let w = world();
    w.save(&verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[("gap-store", "high", STORE)],
    ));
    let asked = WorkflowV2CallRecord::new(
        "run",
        slot(),
        1,
        "h".into(),
        WorkflowV2Result::accepted("slot"),
        vec![],
    );
    let view = with_residual_plan(
        &asked,
        &asked.result,
        &w.store,
        Some(&w.universe),
        Some(w.root()),
    )
    .unwrap();
    let entry = &view.data[RESIDUAL_GAPS_KEY][0];
    assert_eq!(entry["source"], "host");
    assert_eq!(entry["task_ids"], json!(["TASK-A"]));
    assert_eq!(entry["expansion_files"], json!([STORE]));
    assert_eq!(entry["attempted"], false);
    // Any other record, even one carrying the key, never hands one over.
    let mut forged = WorkflowV2Result::accepted("x");
    forged.data = json!({RESIDUAL_GAPS_KEY: [{"source": "host", "key": "residual-forged"}]});
    let other = verdict("x-1", &["TASK-A"], &[]);
    let viewed =
        with_residual_plan(&other, &forged, &w.store, Some(&w.universe), Some(w.root())).unwrap();
    assert!(viewed.data.get(RESIDUAL_GAPS_KEY).is_none());
    assert!(with_residual_plan(&other, &other.result, &w.store, None, None).is_none());
}

/// Issue-117: a verifier that accepts while recording the gap again -- same
/// id, same file or same opening words, at ANY severity -- resolves nothing.
#[test]
fn a_gap_its_judging_verifier_records_again_at_any_severity_stands() {
    for again in [
        ("gap-store", "medium", "reworded entirely"),
        (
            "gap-other",
            "medium",
            "crates/shared/src/store.rs:9 still reads the timeframe",
        ),
        // Prose never says a gap is resolved: "prefixed" and "fixed" alike.
        (
            "gap-other",
            "medium",
            "prefixed ids in crates/shared/src/store.rs:9",
        ),
        (
            "gap-other",
            "medium",
            "crates/shared/src/store.rs:3 split is now fixed and pinned",
        ),
        (
            "gap-other",
            "note",
            "the store lane reads the timeframe segment, not the instrument, still",
        ),
        (
            "gap-other",
            "",
            "the store lane reads the timeframe segment, not the instrument",
        ),
    ] {
        let w = world();
        let recorded = verdict(
            "verification-wave-review-verify-task-a-1-2",
            &["TASK-A"],
            &[(
                "gap-store",
                "high",
                "the store lane reads the timeframe segment, not the instrument, at crates/shared/src/store.rs:3",
            )],
        );
        w.save(&recorded);
        let round = w.plan().rounds[0].clone();
        let mut calls = vec![recorded.call.clone(), slot()];
        calls.extend(round_calls_recording(
            &w,
            &round,
            WorkflowV2Status::Accepted,
            &[again],
        ));
        let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
        assert_eq!(gate.blocking.len(), 1, "{again:?}: {gate:#?}");
        assert!(gate.blocking[0].contains("again"), "{gate:#?}");
    }
    // An unrelated note does not reopen it.
    let w = world();
    let recorded = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[(
            "gap-store",
            "high",
            "the store lane reads the timeframe, crates/shared/src/store.rs:3",
        )],
    );
    w.save(&recorded);
    let round = w.plan().rounds[0].clone();
    let mut calls = vec![recorded.call.clone(), slot()];
    calls.extend(round_calls_recording(
        &w,
        &round,
        WorkflowV2Status::Accepted,
        &[(
            "gap-fmt",
            "low",
            "crates/a/src/lib.rs:1 has a formatting nit",
        )],
    ));
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(gate.blocking.is_empty(), "{gate:#?}");
}

/// Issue-117: five gaps of one task set never overflow a prompt: the group
/// splits, and every round's prompt as the prelude builds it passes the
/// host's own dispatch check.
#[test]
fn five_gaps_of_one_group_split_into_rounds_whose_prompts_pass_dispatch() {
    let w = world();
    let long = |n: usize| {
        format!(
            "{STORE}:{n} gap number {n}: {}",
            "the lane diverges from its twin. ".repeat(30)
        )
    };
    let texts: Vec<String> = (1..=5).map(long).collect();
    let ids: Vec<String> = (1..=5).map(|n| format!("gap-{n}")).collect();
    let gaps: Vec<(&str, &str, &str)> = ids
        .iter()
        .zip(&texts)
        .map(|(id, text)| (id.as_str(), "high", text.as_str()))
        .collect();
    w.save(&verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &gaps,
    ));
    let plan = w.plan();
    assert_eq!(plan.rounds.len(), 2, "{:?}", plan.rounds);
    assert_eq!(
        plan.rounds[0].residuals.len() + plan.rounds[1].residuals.len(),
        5
    );
    for round in &plan.rounds {
        let view = round_view(round, &w.store);
        assert_eq!(view["dispatchable"], true, "{view}");
        // The fix prompt quotes the finding whole, as the prelude does.
        let finding = json!([{"id": round.key, "claim": view["claim"]}]);
        let mut fix = execution(round, true, &["crates/a/src/lib.rs", STORE]);
        fix.call.options.task = Some(format!(
            "Post-review remediation. Findings (verbatim):\n{finding}"
        ));
        assert_eq!(
            residual_refusal(&fix, &w.store, Some(&w.universe), Some(w.root())),
            None
        );
    }
}

/// A file-only match reopens a gap only when it is weighty, unresolved and
/// from a verifier of the round's tasks: the fixing verifier's low note on
/// the same file, a note that says it is resolved, or another task's note
/// resolve it.
#[test]
fn a_fixed_high_gap_whose_verifier_leaves_a_low_note_on_the_same_file_resolves() {
    let recorded_gap = (
        "gap-store",
        "high",
        "the store lane reads the timeframe, crates/shared/src/store.rs:3",
    );
    for note in [(
        "gap-doc",
        "low",
        "add a doc comment in crates/shared/src/store.rs:1",
    )] {
        let w = world();
        let recorded = verdict(
            "verification-wave-review-verify-task-a-1-2",
            &["TASK-A"],
            &[recorded_gap],
        );
        w.save(&recorded);
        let round = w.plan().rounds[0].clone();
        let mut calls = vec![recorded.call.clone(), slot()];
        calls.extend(round_calls_recording(
            &w,
            &round,
            WorkflowV2Status::Accepted,
            &[note],
        ));
        let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
        assert!(gate.blocking.is_empty(), "{note:?}: {gate:#?}");
    }
    // Another task's verifier, after the round, noting the same file.
    let w = world();
    let recorded = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[recorded_gap],
    );
    w.save(&recorded);
    let round = w.plan().rounds[0].clone();
    let mut calls = vec![recorded.call.clone(), slot()];
    calls.extend(round_calls(&w, &round, WorkflowV2Status::Accepted));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let other = verdict(
        "verification-wave-review-verify-task-b-1-30",
        &["TASK-B"],
        &[(
            "gap-b",
            "medium",
            "crates/shared/src/store.rs is read by B too",
        )],
    );
    w.save(&other);
    calls.push(other.call.clone());
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        !gate.blocking.iter().any(|b| b.contains("`gap-store`")),
        "{gate:#?}"
    );
}

/// Every HIGH gap gets a round, however many rounds that takes; only
/// rounds of medium gaps are bounded, the rest reported as warnings.
#[test]
fn every_high_gap_gets_a_round_and_only_medium_rounds_are_capped() {
    let w = world();
    let texts: Vec<String> = (0..60)
        .map(|n| format!("{STORE}:{n} lane gap {n}"))
        .collect();
    let ids: Vec<String> = (0..60).map(|n| format!("gap-{n:02}")).collect();
    let gaps: Vec<(&str, &str, &str)> = ids
        .iter()
        .zip(&texts)
        .enumerate()
        .map(|(n, (id, text))| {
            (
                id.as_str(),
                if n < 30 { "high" } else { "medium" },
                text.as_str(),
            )
        })
        .collect();
    w.save(&verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &gaps,
    ));
    let plan = w.plan();
    let high = plan
        .rounds
        .iter()
        .flat_map(|round| &round.residuals)
        .filter(|residual| residual.severity == ResidualSeverity::High)
        .count();
    assert_eq!(high, 30, "no high gap is left without a round");
    assert!(
        plan.reported
            .iter()
            .all(|(residual, _)| residual.severity == ResidualSeverity::Medium),
        "{:?}",
        plan.reported
    );
    assert!(
        !plan.reported.is_empty(),
        "medium rounds beyond the bound are reported"
    );
}
