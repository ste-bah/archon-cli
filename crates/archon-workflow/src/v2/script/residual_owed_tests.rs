//! The HIGH gaps no earlier pass could plan are planned by the third pass
//! while its cap allows, and reported as the harness cap exhausted when it
//! does not.

use super::super::gate_tests::slot;
use super::super::second_pass_tests::*;
use super::super::tests::*;
use super::super::*;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2Status};
use serde_json::json;


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

fn pause() {
    std::thread::sleep(std::time::Duration::from_millis(5));
}

/// A first-pass round whose verifier REFUSED while recording `gaps`.
fn refused_first_pass(w: &World, gaps: &[(&str, &str, &str)]) -> Vec<WorkflowV2HostCall> {
    let (recorded, first) = first_round(w, "high");
    let fix = round_call(&first, "review-remediate-residual-3", "remediate", 1);
    w.save(&record(
        fix.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    pause();
    let verify = record(
        round_call(
            &first,
            "verification-wave-review-verify-residual-4",
            "verify",
            1,
        ),
        WorkflowV2Status::NeedsReview,
        &["TASK-A"],
        gaps,
    );
    w.save(&verify);
    vec![
        recorded.call.clone(),
        slot(),
        fix,
        verify.call.clone(),
        second_slot(),
    ]
}

#[test]
fn a_refused_first_pass_verifiers_high_gap_is_planned_by_the_third_pass() {
    let w = package_world();
    let regression = (
        "gap-b-lane",
        "high",
        "crates/b/src/lib.rs:4 drops the lane's version",
    );
    let mut calls = refused_first_pass(&w, &[regression]);
    // No second-pass round carries it.
    assert!(second(&w).rounds.is_empty(), "{:?}", second(&w).rounds);
    let plan = third(&w);
    assert_eq!(plan.rounds.len(), 1, "{plan:?}");
    assert_eq!(ids(&plan.rounds[0].tasks), ["TASK-B"]);
    assert_eq!(plan.rounds[0].residuals[0].id, "gap-b-lane");
    // Unresolved, it blocks as the round that did not resolve it.
    calls.push(third_slot());
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking
            .iter()
            .any(|b| b.contains("gap-b-lane") && b.contains("did not resolve it")),
        "{gate:#?}"
    );
}

/// No plannable HIGH gap is left unplanned while the cap allows: every
/// owed gap gets a round up to the cap, and the rest -- and a gap recorded
/// after the last pass -- block as the harness cap exhausted.
#[test]
fn every_owed_high_gap_is_planned_up_to_the_cap_and_the_rest_block_as_cap_exhausted() {
    let w = package_world();
    let gaps = [
        (
            "gap-b-lane",
            "high",
            "crates/b/src/lib.rs:4 drops the lane's version",
        ),
        (
            "gap-pathless",
            "high",
            "the lanes disagree about what a version is",
        ),
        ("gap-a-lane", "high", "crates/a/src/lib.rs:2 drops the lane"),
    ];
    let mut calls = refused_first_pass(&w, &gaps);
    let plan = third(&w);
    let carried: Vec<&str> = plan
        .rounds
        .iter()
        .flat_map(|r| r.residuals.iter().map(|g| g.id.as_str()))
        .collect();
    assert_eq!(plan.rounds.len(), MAX_THIRD_PASS_ROUNDS, "{plan:?}");
    // Nothing plannable was left out while a round was free.
    assert_eq!(carried.len() + plan.reported.len(), gaps.len(), "{plan:?}");
    assert!(
        plan.reported
            .iter()
            .all(|(_, why)| why.starts_with("harness cap exhausted")),
        "{:?}",
        plan.reported
    );
    calls.push(third_slot());
    // A HIGH gap an acceptance-stage verifier records after the last pass.
    let late = verdict(
        "verification-wave-review-verify-task-a-1-20",
        &["TASK-A"],
        &[("gap-late", "high", "crates/a/src/lib.rs:9 regressed")],
    );
    w.save(&late);
    calls.push(late.call.clone());
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    for (id, _, _) in gaps.iter().chain([&("gap-late", "", "")]) {
        let clause = gate.blocking.iter().find(|b| b.contains(id));
        let clause = clause.unwrap_or_else(|| panic!("{id} does not block: {gate:#?}"));
        assert!(
            clause.contains("did not resolve it") || clause.contains("harness cap exhausted"),
            "{id}: {clause}"
        );
    }
}
