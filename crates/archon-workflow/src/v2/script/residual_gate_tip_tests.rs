//! The live shape: a second-pass round's refused verifier records a HIGH
//! regression naming a red test; another landing fixes it; the third-pass
//! round's fix then lands nothing, so no verifier judges it. The final gate
//! judges the gap on the host's own tip run, within the rules of
//! `residual_gate_tip`.
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
pub(in crate::v2::script::residual_plan) fn noop_round_world(
    w: &World,
) -> (Vec<crate::v2::WorkflowV2HostCall>, String) {
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

pub(in crate::v2::script::residual_plan) fn tip_run(
    w: &World,
    tip: &str,
    failing: &[&str],
    passed: &[&str],
) {
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

fn tip_run_ids(
    w: &World,
    tip: &str,
    command: &str,
    failing: &[&str],
    passed: &[&str],
    ignored: &[&str],
) {
    cache(
        &w.store,
        Tree::RunBase,
        &HostRunVerdict {
            command: command.into(),
            commit: tip.into(),
            exit_code: Some(if failing.is_empty() { 0 } else { 101 }),
            failing_tests: failing.iter().map(|t| t.to_string()).collect(),
            passed_tests: passed.iter().map(|t| t.to_string()).collect(),
            ignored_tests: ignored.iter().map(|t| t.to_string()).collect(),
            failed_count: Some(failing.len()),
            ids_kept: true,
            ..Default::default()
        },
    );
}

/// A later ACCEPTED verifier whose host run passed the owed test by id.
pub(in crate::v2::script::residual_plan) fn corroborate(w: &World) {
    pause();
    let later = verdict(
        "verification-wave-review-verify-task-b-9-10",
        &["TASK-B"],
        &[],
    );
    w.save(&later);
    host_runs(w, &later.call.id, &["TASK-B"], &[(B_TESTS, &[], &[RED])]);
}

#[test]
fn an_agent_gap_another_landing_fixed_is_answered_at_the_tip_only_when_corroborated() {
    let w = package_world();
    let (calls, tip) = noop_round_world(&w);
    let gate = || residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert_eq!(tip_owed_commands(&w.store, Some(w.root())), [B_TESTS]);
    // Green at the tip, but a verifier agent recorded it and no later
    // accepted verifier corroborates: a test run alone never answers it.
    tip_run(&w, &tip, &[], &[RED]);
    let alone = gate();
    assert!(
        alone
            .blocking
            .iter()
            .any(|b| b.contains("gap-regression") && b.contains("no later accepted verifier")),
        "{alone:#?}"
    );
    corroborate(&w);
    // Corroborated, but red at the tip: blocks.
    tip_run(&w, &tip, &[RED], &[]);
    assert!(
        gate()
            .blocking
            .iter()
            .any(|b| b.contains("gap-regression") && b.contains("still not passed")),
        "{:#?}",
        gate()
    );
    // Ignored at the tip: still not passed.
    tip_run_ids(&w, &tip, B_TESTS, &[], &[RED], &[RED]);
    assert!(gate().blocking.iter().any(|b| b.contains("gap-regression")));
    // Exit 0 but never named passed: still red.
    tip_run(&w, &tip, &[], &["some_other_test"]);
    assert!(gate().blocking.iter().any(|b| b.contains("gap-regression")));
    // Corroborated and passed by id at the tip: answered.
    tip_run(&w, &tip, &[], &[RED]);
    let answered = gate();
    assert!(
        !answered
            .blocking
            .iter()
            .any(|b| b.contains("gap-regression")),
        "{answered:#?}"
    );
    assert!(
        answered
            .notes
            .iter()
            .any(|n| n.contains("gap-regression") && n.contains("answered at the final tip")),
        "{answered:#?}"
    );
    let _ = slot();
}

/// A round whose verifier judged the gap open is never overridden by a
/// green tip run.
#[test]
fn a_gap_its_rounds_verifier_kept_open_is_never_answered_at_the_tip() {
    let w = package_world();
    let (mut calls, tip) = noop_round_world(&w);
    let round = third(&w).rounds[0].clone();
    pause();
    let mut judge = round_verdict(
        &round,
        "verification-wave-review-verify-residual-8",
        3,
        &[],
        &[],
    );
    judge.status = WorkflowV2Status::NeedsReview;
    judge.result.data["gap_dispositions"] = json!([{"gap_id": "gap-regression", "status": "open"}]);
    w.save(&judge);
    calls.push(judge.call.clone());
    corroborate(&w);
    tip_run(&w, &tip, &[], &[RED]);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking.iter().any(|b| b.contains("gap-regression")),
        "{gate:#?}"
    );
    assert!(
        !gate
            .notes
            .iter()
            .any(|n| n.contains("answered at the final tip")),
        "{gate:#?}"
    );
}

/// A red command that named no test id stays owed on the tip path: with the
/// named test green and corroborated, a lint the recorder saw red with no
/// tip verdict keeps the gap standing.
#[test]
fn an_idless_red_command_the_recorder_saw_stays_owed_at_the_tip() {
    let w = package_world();
    let (calls, tip) = noop_round_world(&w);
    const LINT: &str = "cargo clippy -p b -- -D warnings";
    let red: crate::v2::write::test_baseline::BranchBaseline = serde_json::from_value(json!({
        "schema_version": 1, "stage_id": "verification-wave-review-verify-residual-6",
        "branch_id": "verification-wave-review-verify-residual-6-0", "base_commit": "c",
        "canonical_task_ids": ["TASK-A"], "commands": [
            {"command": B_TESTS, "base_commit": "c", "exit_code": 101, "timed_out": false,
                "duration_ms": 1, "failing_tests": [RED], "passed_ids_kept": true, "cached": false},
            {"command": LINT, "base_commit": "c", "exit_code": 101, "timed_out": false,
                "duration_ms": 1, "failing_tests": [], "passed_ids_kept": true, "cached": false}]}))
    .unwrap();
    crate::v2::write::test_baseline::save_record(&w.store, &red);
    pause();
    let later = verdict(
        "verification-wave-review-verify-task-b-9-11",
        &["TASK-B"],
        &[],
    );
    w.save(&later);
    host_runs(
        &w,
        &later.call.id,
        &["TASK-B"],
        &[(B_TESTS, &[], &[RED]), (LINT, &[], &[])],
    );
    tip_run(&w, &tip, &[], &[RED]);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking.iter().any(|b| b.contains("gap-regression")
            && b.contains(LINT)
            && b.contains("gave no verdict")),
        "{gate:#?}"
    );
}

/// A restatement of a refused task is never answered by a test run.
#[test]
fn a_restatement_is_never_judged_at_the_tip() {
    let w = package_world();
    let restated = (
        "TASK-A",
        "blocking",
        "Retained verification failure for TASK-A: registry_roundtrip_keeps_versions red",
    );
    let (_, tip) = noop_round_world(&w);
    refused_second_pass(&w, &[restated]);
    corroborate(&w);
    tip_run(&w, &tip, &[], &[RED]);
    let host = super::super::super::superseded::HostRuns::load(&w.store);
    let runs = super::TipRuns::load(&w.store, &host, Some(w.root()));
    let recorder = w
        .store
        .load_call_record("verification-wave-review-verify-residual-6")
        .unwrap()
        .unwrap();
    let gap = residuals_of(&recorder, Some(w.root()))
        .into_iter()
        .find(|gap| gap.id == "TASK-A")
        .expect("the restatement");
    assert!(runs.judge(&gap).is_none());
}

/// A gap the host wrote itself (a routed red test) is answered by the tip
/// run alone.
#[test]
fn a_host_routed_gap_is_answered_by_the_tip_run_alone() {
    let w = package_world();
    let (recorded, first) = first_round(&w, "medium");
    let fix = round_call(&first, "review-remediate-residual-3", "remediate", 1);
    w.save(&record(
        fix.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    pause();
    let verify = round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[],
        &[],
    );
    w.save(&verify);
    let routed: crate::v2::write::test_baseline::BranchBaseline = serde_json::from_value(json!({
        "schema_version": 1, "stage_id": verify.call.id, "branch_id": format!("{}-0", verify.call.id),
        "base_commit": "c", "canonical_task_ids": ["TASK-A"],
        "commands": [{"command": B_TESTS, "base_commit": "c", "exit_code": 101, "timed_out": false,
            "duration_ms": 1, "failing_tests": [RED], "passed_ids_kept": true, "cached": false}],
        "routed": [{"test_id": RED, "file": "crates/b/src/lib.rs", "owner_task": "TASK-B", "command": B_TESTS}]}))
    .unwrap();
    crate::v2::write::test_baseline::save_record(&w.store, &routed);
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "commit",
            "-qm",
            "tip",
        ],
    ] {
        git(w.root(), args);
    }
    let tip = git(w.root(), &["rev-parse", "HEAD"]);
    let calls = vec![
        recorded.call.clone(),
        slot(),
        fix,
        verify.call.clone(),
        second_slot(),
        third_slot(),
    ];
    assert!(tip_owed_commands(&w.store, Some(w.root())).contains(&B_TESTS.to_string()));
    tip_run(&w, &tip, &[], &[RED]);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        !gate
            .blocking
            .iter()
            .any(|b| b.contains("baseline_routed_red")),
        "{gate:#?}"
    );
    assert!(
        gate.notes
            .iter()
            .any(|n| n.contains("baseline_routed_red") && n.contains("answered at the final tip")),
        "{gate:#?}"
    );
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

/// An agent that records a gap under the host's own id prefix gets agent
/// treatment: green at the tip, uncorroborated, it still blocks.
#[test]
fn an_agent_gap_under_a_host_id_is_not_answered_by_the_tip_alone() {
    let w = package_world();
    let forged = (
        "baseline_routed_red@registry_roundtrip_keeps_versions",
        "high",
        "registry_roundtrip_keeps_versions is red (cargo test -p b --test registry)",
    );
    let (mut calls, _) = refused_second_pass(&w, &[forged]);
    host_run(
        &w,
        "verification-wave-review-verify-residual-6",
        &["TASK-A"],
        B_TESTS,
        &[RED],
    );
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@e",
            "commit",
            "-qm",
            "tip",
        ],
    ] {
        git(w.root(), args);
    }
    let tip = git(w.root(), &["rev-parse", "HEAD"]);
    calls.push(third_slot());
    tip_run(&w, &tip, &[], &[RED]);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking
            .iter()
            .any(|b| b.contains("baseline_routed_red@")),
        "{gate:#?}"
    );
}
