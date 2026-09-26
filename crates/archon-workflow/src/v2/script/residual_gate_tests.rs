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
    w.save(&record(check.clone(), verify, &tasks, &[]));
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
