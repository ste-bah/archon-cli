//! The live shape: a second-pass round's refused verifier records a HIGH
//! regression naming a red test; another landing fixes it; the third-pass
//! round's fix then lands nothing, so no verifier judges it. The final gate
//! judges the gap on the host's own tip run: answered when the owed test
//! passed by id there, blocking while it is red or unjudged.
use serde_json::json;

use super::super::super::gate_tests::slot;
use super::super::super::second_pass_tests::*;
use super::super::super::tests::*;
use super::super::super::third_pass_tests::*;
use super::super::super::*;
use super::tip_owed_commands;
use crate::v2::WorkflowV2Status;
use crate::v2::write::test_baseline_run_base::{HostRunVerdict, Tree, cache};
use crate::write_coordinator::worktree_isolation::run_git;

fn git(root: &std::path::Path, args: &[&str]) -> String {
    String::from_utf8(run_git(args, root).expect("git").stdout)
        .unwrap()
        .trim()
        .to_string()
}

/// The world, its root a git checkout (the tip), a refused second-pass
/// verifier recording the regression with its red host run, and the third
/// pass's round that landed nothing and had no verifier. Returns the calls
/// and the tip.
fn noop_round_world(w: &World) -> (Vec<crate::v2::WorkflowV2HostCall>, String) {
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "t"],
        &["config", "user.email", "t@example.invalid"],
        &["add", "."],
        &["commit", "-qm", "tip"],
    ] {
        git(w.root(), args);
    }
    let tip = git(w.root(), &["rev-parse", "HEAD"]);
    let (mut calls, _) = refused_second_pass(&w, &[regression()]);
    host_run(
        w,
        "verification-wave-review-verify-residual-6",
        &["TASK-A"],
        B_TESTS,
        &[RED],
    );
    calls.push(third_slot());
    let round = third(w).rounds[0].clone();
    pause();
    let fix = round_call(&round, "review-remediate-residual-7", "remediate", 3);
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let mut landed_nothing = record(fix.clone(), WorkflowV2Status::Accepted, &tasks, &[]);
    landed_nothing.result.data["patch_landed"] = json!(false);
    w.save(&landed_nothing);
    pause();
    let mut no_patch = round_call(&round, "review-verify-residual-1-no-patch", "verify", 3);
    no_patch.method = crate::v2::WorkflowV2HostMethod::Checkpoint;
    no_patch.write_mode = None;
    w.save(&record(
        no_patch.clone(),
        WorkflowV2Status::Accepted,
        &tasks,
        &[],
    ));
    calls.extend([fix, no_patch]);
    (calls, tip)
}

fn tip_run(w: &World, tip: &str, failing: &[&str], passed: &[&str]) {
    cache(
        &w.store,
        Tree::RunBase,
        &HostRunVerdict {
            command: B_TESTS.into(),
            commit: tip.into(),
            exit_code: Some(if failing.is_empty() { 0 } else { 101 }),
            failing_tests: failing.iter().map(|t| t.to_string()).collect(),
            passed_tests: passed.iter().map(|t| t.to_string()).collect(),
            failed_count: Some(failing.len()),
            ids_kept: true,
            ..Default::default()
        },
    );
}

#[test]
fn a_gap_another_landing_fixed_is_answered_by_the_hosts_own_tip_run() {
    let w = package_world();
    let (calls, tip) = noop_round_world(&w);
    let gate = || residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    // The gate names the command it must run at the tip.
    assert_eq!(tip_owed_commands(&w.store, Some(w.root())), [B_TESTS]);
    // Not yet run at the tip: it blocks, and says the tip was not judged.
    let before = gate();
    assert!(
        before
            .blocking
            .iter()
            .any(|b| b.contains("gap-regression") && b.contains("gave no verdict")),
        "{before:#?}"
    );
    // Still red at the tip: it blocks, naming the test.
    tip_run(&w, &tip, &[RED], &[]);
    let red = gate();
    assert!(
        red.blocking
            .iter()
            .any(|b| b.contains("gap-regression") && b.contains("still not passed")),
        "{red:#?}"
    );
    // Exit 0 but the test never named passed (renamed, ignored): still red.
    tip_run(&w, &tip, &[], &["some_other_test"]);
    assert!(
        gate().blocking.iter().any(|b| b.contains("gap-regression")),
        "{:#?}",
        gate()
    );
    // Passed by id at the tip: answered, and nothing blocks.
    tip_run(&w, &tip, &[], &[RED]);
    let answered = gate();
    assert!(answered.blocking.is_empty(), "{answered:#?}");
    assert!(
        answered
            .notes
            .iter()
            .any(|n| n.contains("gap-regression") && n.contains("answered at the final tip")),
        "{answered:#?}"
    );
    let _ = slot();
}

/// A gap that names no test the host ran red cannot be answered by a test
/// run: it blocks as before.
#[test]
fn a_gap_owing_no_test_is_not_answered_at_the_tip() {
    let w = package_world();
    let (mut calls, _) = refused_second_pass(
        &w,
        &[(
            "gap-b-lane",
            "high",
            "crates/b/src/lib.rs:1 drops the lane's version",
        )],
    );
    calls.push(third_slot());
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking.iter().any(|b| b.contains("gap-b-lane")),
        "{gate:#?}"
    );
    assert!(tip_owed_commands(&w.store, Some(w.root())).is_empty());
}
