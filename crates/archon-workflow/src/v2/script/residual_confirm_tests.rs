//! The live shape: a third-pass round's fix landed nothing and no verifier
//! judged it; the one host-planned confirmation is asked, and its verdict is
//! the round's.
use serde_json::{Value, json};

use super::super::super::gate::tip::tests::{corroborate, noop_round_world, tip_run};
use super::super::super::second_pass_tests::*;
use super::super::super::tests::*;
use super::super::super::third_pass_tests::*;
use super::super::super::*;
use super::{confirmation_call_id, confirmation_claim, confirmation_view};
use crate::v2::{WorkflowV2CallExecution, WorkflowV2HostMethod, WorkflowV2Status};

/// The round, recorded done (as the prelude records a returned round).
fn done_round(w: &World) -> (Vec<crate::v2::WorkflowV2HostCall>, PlannedRound) {
    let (mut calls, _) = noop_round_world(w);
    let round = third(w).rounds[0].clone();
    let done = call(&done_checkpoint_id(&round.key), json!({}), false);
    let mut done = done;
    done.method = WorkflowV2HostMethod::Checkpoint;
    done.options.extra.clear();
    w.save(&record(done.clone(), WorkflowV2Status::Accepted, &[], &[]));
    calls.push(done);
    (calls, round)
}

fn confirmation(w: &World, round: &PlannedRound) -> WorkflowV2CallExecution {
    let entry = confirmation_view(&w.store, Some(&w.universe), Some(w.root())).unwrap();
    let entry = entry
        .into_iter()
        .find(|entry| entry["key"] == round.key.as_str())
        .expect("the round is listed");
    let mut options = crate::v2::WorkflowV2HostOptions::default();
    options
        .extra
        .insert("remediationContract".into(), entry["contract"].clone());
    options.task = Some(format!(
        "{}\nBaseline rule: ...",
        entry["claim"].as_str().unwrap()
    ));
    WorkflowV2CallExecution {
        call: crate::v2::WorkflowV2HostCall {
            id: confirmation_call_id(&round.key),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        input: json!({"source_data": [{"canonical_task_ids": round.tasks}]}),
        depends_on: vec![],
    }
}

fn dispositions(round: &PlannedRound, status: &str) -> Value {
    round
        .residuals
        .iter()
        .map(|gap| json!({"gap_id": gap.id, "status": status}))
        .collect()
}

#[test]
fn an_unjudged_no_op_round_gets_one_confirmation_whose_verdict_is_the_rounds() {
    let w = package_world();
    let (mut calls, round) = done_round(&w);
    calls.push(third_slot());
    let before_keys: Vec<String> = third(&w).rounds.iter().map(|r| r.key.clone()).collect();
    // Before it is asked, the gate blocks and says no verifier judged it.
    let gate = || residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(gate().blocking.iter().any(|b| b.contains("gap-regression")));
    // Listed once, with the host's claim, answered by the dispatch check.
    let execution = confirmation(&w, &round);
    assert!(confirmation_claim(&round).contains("gap-regression"));
    assert_eq!(
        residual_refusal(&execution, &w.store, Some(&w.universe), Some(w.root())),
        None
    );
    let mut forged = execution.clone();
    forged.call.id = "verification-wave-other".into();
    assert!(residual_refusal(&forged, &w.store, Some(&w.universe), Some(w.root())).is_some());
    // Accepted with every gap resolved: the round resolves, nothing blocks.
    pause();
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let mut verdict = record(
        execution.call.clone(),
        WorkflowV2Status::Accepted,
        &tasks,
        &[],
    );
    verdict.result.data["gap_dispositions"] = dispositions(&round, "resolved");
    w.save(&verdict);
    let mut with = calls.clone();
    with.push(execution.call.clone());
    let decided = residual_verdict(&with, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        !decided
            .blocking
            .iter()
            .any(|b| b.contains("gap-regression")),
        "{decided:#?}"
    );
    // Asked once: listed as attempted, and its record replays the check.
    let listed = confirmation_view(&w.store, Some(&w.universe), Some(w.root())).unwrap();
    assert_eq!(listed[0]["attempted"], json!(true));
    assert_eq!(
        residual_refusal(&execution, &w.store, Some(&w.universe), Some(w.root())),
        None
    );
    // No pass's plan moved.
    let after_keys: Vec<String> = third(&w).rounds.iter().map(|r| r.key.clone()).collect();
    assert_eq!(before_keys, after_keys);
}

#[test]
fn a_refused_confirmation_blocks_and_the_tip_never_overrides_it() {
    let w = package_world();
    let (mut calls, round) = done_round(&w);
    calls.push(third_slot());
    let execution = confirmation(&w, &round);
    pause();
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let mut verdict = record(
        execution.call.clone(),
        WorkflowV2Status::NeedsReview,
        &tasks,
        &[],
    );
    verdict.result.data["gap_dispositions"] = dispositions(&round, "open");
    w.save(&verdict);
    corroborate(&w);
    let tip = crate::repository_record::git_head(w.root()).unwrap();
    tip_run(&w, &tip, &[], &[RED]);
    calls.push(execution.call.clone());
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking.iter().any(|b| b.contains("gap-regression")),
        "{gate:#?}"
    );
    assert!(
        !gate
            .notes
            .iter()
            .any(|n| n.contains("answered at the final tip"))
    );
}
