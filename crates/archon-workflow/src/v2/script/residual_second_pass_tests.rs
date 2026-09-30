//! Issue-118: the second residual pass plans rounds for what the first
//! pass's own rounds found, is bounded, and the final gate judges them.

use super::gate_tests::slot;
use super::tests::*;
use super::*;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2Status};
use serde_json::json;

const TESTS: &str = "crates/shared/src/store_tests.rs";
const STALE: &str = "store::tests::stale";
const LIB: &str = "cargo test -p shared --lib";

/// The Issue-117 world plus a package the host can resolve test ids in.
pub(super) fn package_world() -> World {
    let w = world();
    std::fs::write(
        w.root().join("crates/shared/Cargo.toml"),
        "[package]\nname = \"shared\"\n",
    )
    .unwrap();
    std::fs::write(w.root().join(TESTS), "//\n").unwrap();
    w
}

/// A first-pass round: an accepted verifier's gap on the store file.
pub(super) fn first_round(w: &World, severity: &str) -> (WorkflowV2CallRecord, PlannedRound) {
    let recorded = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[(
            "gap-store",
            severity,
            &format!("{STORE}:9 reads the timeframe"),
        )],
    );
    w.save(&recorded);
    let round = w.plan().rounds[0].clone();
    (recorded, round)
}

pub(super) fn second_slot() -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert(RESIDUAL_GAPS_MARKER.into(), Value::Bool(true));
    options.extra.insert(RESIDUAL_PASS_KEY.into(), json!(2));
    WorkflowV2HostCall {
        id: "residual-gaps-2".into(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    }
}

/// A call of `round` at `stage`, as the prelude files it.
pub(super) fn round_call(
    round: &PlannedRound,
    id: &str,
    stage: &str,
    pass: u64,
) -> WorkflowV2HostCall {
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let files: Vec<&str> = round.files.iter().map(String::as_str).collect();
    let mut residual = json!({"key": round.key, "files": files});
    if pass >= 2 {
        residual["pass"] = json!(pass);
    }
    let mut contract = contract(
        stage,
        &tasks,
        json!({"residual": residual, "contest": round.key}),
    );
    contract["maxRounds"] = json!(1);
    call(id, contract, stage == "remediate")
}

/// A verifier of `round` the host refused over `red` (the shape of
/// wf-0ddadd81's round-3 verifier), or accepted recording `gaps`.
pub(super) fn round_verdict(
    round: &PlannedRound,
    id: &str,
    pass: u64,
    red: &[&str],
    gaps: &[(&str, &str, &str)],
) -> WorkflowV2CallRecord {
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let status = if red.is_empty() {
        WorkflowV2Status::Accepted
    } else {
        WorkflowV2Status::NeedsReview
    };
    let mut verdict = record(round_call(round, id, "verify", pass), status, &tasks, gaps);
    verdict.result.data["outcomes"][0]["result"]["data"] = json!({"baseline_red_tests": red});
    verdict.result.data["outcomes"][0]["result"]["commands_run"] =
        json!([{"kind": "test", "command": LIB, "status": "failed", "output_summary": "7 failed"}]);
    verdict
}

pub(super) fn second(w: &World) -> ResidualPlan {
    let records = session_records(&w.store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    second_pass_plan(&refs, &w.store, Some(&w.universe), Some(w.root()))
}

#[test]
fn a_refused_rounds_red_tests_outside_its_scope_get_one_second_pass_round() {
    let w = package_world();
    let (_, first) = first_round(&w, "medium");
    let before: Vec<String> = w.plan().rounds.iter().map(|r| r.key.clone()).collect();
    w.save(&round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[STALE],
        &[],
    ));
    // The first pass is exactly what it was: the refused verifier is one of
    // its own rounds', never its population.
    let after: Vec<String> = w.plan().rounds.iter().map(|r| r.key.clone()).collect();
    assert_eq!(before, after);
    let plan = second(&w);
    assert!(plan.reported.is_empty(), "{:?}", plan.reported);
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.rounds);
    let round = &plan.rounds[0];
    assert_eq!(round.kind, RoundKind::Expansion);
    assert!(round.key.ends_with("p2") && round.key != first.key);
    // The refused round again, whole: its own file and gap, plus the test's
    // file (its parent module's is the round's own store file).
    assert_eq!(ids(&round.files), [STORE, TESTS]);
    assert_eq!(ids(&round.tasks), ["TASK-A"]);
    let carried: Vec<&str> = round.residuals.iter().map(|g| g.id.as_str()).collect();
    assert_eq!(carried[0], "gap-store", "{carried:?}");
    let gap = &round.residuals[1];
    assert!(gap.id.starts_with(REFUSED_RED_GAP_ID), "{}", gap.id);
    assert!(gap.description.contains(STALE), "{}", gap.description);
    assert_eq!(gap.files, [STORE, TESTS]);
}

/// A first-pass round with a HIGH gap whose verifier the host refused over
/// a test it could not write, then its second-pass retry with `verify`.
fn refused_then_retried(w: &World, verify: WorkflowV2Status) -> Vec<WorkflowV2HostCall> {
    let (recorded, first) = first_round(w, "high");
    let fix1 = round_call(&first, "review-remediate-residual-3", "remediate", 1);
    w.save(&record(
        fix1.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let verify1 = round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[STALE],
        &[],
    );
    w.save(&verify1);
    let retry = second(w).rounds[0].clone();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let fix2 = round_call(&retry, "review-remediate-residual-5", "remediate", 2);
    w.save(&record(
        fix2.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let verify2 = round_call(
        &retry,
        "verification-wave-review-verify-residual-6",
        "verify",
        2,
    );
    w.save(&record(verify2.clone(), verify, &["TASK-A"], &[]));
    vec![
        recorded.call.clone(),
        slot(),
        fix1,
        verify1.call.clone(),
        second_slot(),
        fix2,
        verify2,
    ]
}

#[test]
fn a_refused_high_round_blocks_until_its_retry_resolves_it() {
    let w = package_world();
    let calls = refused_then_retried(&w, WorkflowV2Status::Accepted);
    // Without the second slot the refused round's HIGH gap blocks.
    let first_only: Vec<WorkflowV2HostCall> = calls[..4].to_vec();
    let old = residual_verdict(&first_only, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        old.blocking.iter().any(|b| b.contains("gap-store")),
        "{old:#?}"
    );
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(gate.blocking.is_empty(), "{gate:#?}");
    assert!(
        gate.notes
            .iter()
            .any(|n| n.contains("p2") && n.contains("resolved")),
        "{gate:#?}"
    );
    let w = package_world();
    let calls = refused_then_retried(&w, WorkflowV2Status::NeedsReview);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        gate.blocking.iter().any(|b| b.contains("gap-store")),
        "{gate:#?}"
    );
}

/// Batch O: it is the round's own failure, so the round is planned again,
/// whole and with its judgment, and no red gap is added.
#[test]
fn a_red_test_inside_the_refused_rounds_scope_is_its_own_failure_and_retried_whole() {
    let w = package_world();
    // The first pass granted the round the test file itself.
    let recorded = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[(
            "gap-store",
            "medium",
            &format!("{STORE}:9 and {TESTS}:3 disagree"),
        )],
    );
    w.save(&recorded);
    let first = w.plan().rounds[0].clone();
    assert_eq!(ids(&first.files), [STORE, TESTS]);
    w.save(&round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[STALE],
        &[],
    ));
    let plan = second(&w);
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.rounds);
    assert_eq!(plan.rounds[0].residuals, first.residuals);
    assert_eq!(plan.rounds[0].files, first.files);
    assert!(plan.rounds[0].refusal.is_some());
}

#[test]
fn a_host_red_gap_on_any_verdict_and_a_new_high_gap_of_a_round_are_planned() {
    let w = package_world();
    let (_, first) = first_round(&w, "medium");
    let host_gap =
        format!("`{STALE}` in {TESTS} implicating {TESTS}, {STORE}: red at the run base");
    w.save(&round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[],
        &[
            // The round's own gap again: the gate's to judge, not a new round.
            (
                "gap-store",
                "high",
                &format!("{STORE}:9 reads the timeframe"),
            ),
            (
                "gap-new",
                "high",
                "crates/b/src/lib.rs drops the lane's version",
            ),
            ("gap-note", "medium", "crates/b/src/lib.rs could log more"),
        ],
    ));
    // A refused per-task verdict carrying the host's own record of an
    // excused test: owed whatever the verdict.
    let mut refused = verdict(
        "verification-wave-review-verify-task-b-1-3",
        &["TASK-B"],
        &[("baseline_unowned_red_tests-0123abcd", "medium", &host_gap)],
    );
    refused.status = WorkflowV2Status::NeedsReview;
    w.save(&refused);
    let plan = second(&w);
    let carried: Vec<&str> = plan
        .rounds
        .iter()
        .flat_map(|r| r.residuals.iter().map(|g| g.id.as_str()))
        .collect();
    assert!(carried.contains(&"gap-new"), "{carried:?}");
    assert!(
        carried.contains(&"baseline_unowned_red_tests-0123abcd"),
        "{carried:?}"
    );
    // The round's own gap, recorded again exactly, is the gate's to judge;
    // a new MEDIUM gap is planned like a high one (Batch O).
    assert!(!carried.contains(&"gap-store"), "{carried:?}");
    assert!(carried.contains(&"gap-note"), "{carried:?}");
}

#[test]
fn the_second_pass_is_uncapped_and_never_reads_its_own_rounds() {
    let mut w = package_world();
    // Six new high gaps, each on a file of a task set of its own.
    for n in 0..6 {
        let file = format!("crates/x{n}/src/lib.rs");
        std::fs::create_dir_all(w.root().join(format!("crates/x{n}/src"))).unwrap();
        std::fs::write(w.root().join(&file), "//\n").unwrap();
        w.universe
            .tasks
            .push(crate::task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: format!("TASK-X{n}"),
                source_path: "tasks/TASK-B.md".into(),
                files_expected_to_change: vec![file],
                ..Default::default()
            });
    }
    let (_, first) = first_round(&w, "medium");
    let gaps: Vec<(String, String)> = (0..6)
        .map(|n| {
            (
                format!("gap-{n}"),
                format!("crates/x{n}/src/lib.rs:1 is wrong"),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, &str)> = gaps
        .iter()
        .map(|(id, text)| (id.as_str(), "high", text.as_str()))
        .collect();
    w.save(&round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[],
        &borrowed,
    ));
    let plan = second(&w);
    // Batch O: every round is planned; no cap turns one into a report.
    assert_eq!(plan.rounds.len(), 6);
    assert!(plan.reported.is_empty(), "{:?}", plan.reported);
    // A second-pass round's own verifier recording yet another gap changes
    // nothing: its records are never this pass's population.
    let keys: Vec<String> = plan.rounds.iter().map(|r| r.key.clone()).collect();
    w.save(&round_verdict(
        &plan.rounds[0],
        "verification-wave-review-verify-residual-9",
        2,
        &[],
        &[("gap-later", "high", "crates/b/src/lib.rs is still wrong")],
    ));
    let again: Vec<String> = second(&w).rounds.iter().map(|r| r.key.clone()).collect();
    assert_eq!(keys, again);
}

#[test]
fn the_gate_judges_second_pass_rounds_and_plans_no_third() {
    let w = package_world();
    let (recorded, first) = first_round(&w, "medium");
    let fix1 = round_call(&first, "review-remediate-residual-3", "remediate", 1);
    w.save(&record(
        fix1.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    // The first round's verifier accepted, recording a NEW high gap.
    let verify1 = round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[],
        &[(
            "gap-new",
            "high",
            "crates/b/src/lib.rs drops the lane's version",
        )],
    );
    w.save(&verify1);
    let mut calls = vec![recorded.call.clone(), slot(), fix1, verify1.call.clone()];
    // Without the second slot (an older script) the new high gap blocks.
    let old = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        old.blocking.iter().any(|b| b.contains("gap-new")),
        "{old:#?}"
    );
    calls.push(second_slot());
    let plan = second(&w);
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.rounds);
    let round = plan.rounds[0].clone();
    // Asked but not yet run: the gap stands, now because its round did not
    // resolve it.
    let pending = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        pending
            .blocking
            .iter()
            .any(|b| b.contains("gap-new") && b.contains("did not resolve")),
        "{pending:#?}"
    );
    std::thread::sleep(std::time::Duration::from_millis(5));
    let fix2 = round_call(&round, "review-remediate-residual-5", "remediate", 2);
    w.save(&record(
        fix2.clone(),
        WorkflowV2Status::Accepted,
        &["TASK-B"],
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    // Its verifier accepts, and records another high gap: no third pass.
    let verify2 = round_verdict(
        &round,
        "verification-wave-review-verify-residual-6",
        2,
        &[],
        &[("gap-third", "high", "crates/a/src/lib.rs lost a field")],
    );
    w.save(&verify2);
    calls.extend([fix2, verify2.call.clone()]);
    let gate = residual_verdict(&calls, &w.store, Some(&w.universe), Some(w.root()));
    assert!(
        !gate.blocking.iter().any(|b| b.contains("gap-new")),
        "{gate:#?}"
    );
    assert!(
        gate.notes
            .iter()
            .any(|n| n.contains(&round.key) && n.contains("resolved")),
        "{gate:#?}"
    );
    assert!(
        gate.blocking
            .iter()
            .any(|b| b.contains("gap-third") && b.contains("after the second residual pass")),
        "{gate:#?}"
    );
    // And the dispatch check answers the second pass's round, marked as
    // one, and no other.
    let exec = |pass: u64| crate::v2::WorkflowV2CallExecution {
        call: {
            let mut c = round_call(
                &round,
                "verification-wave-review-verify-residual-6",
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
        residual_refusal(&exec(2), &w.store, Some(&w.universe), Some(w.root())),
        None
    );
    assert!(
        residual_refusal(&exec(1), &w.store, Some(&w.universe), Some(w.root()))
            .unwrap()
            .contains("no round of the host's plan")
    );
}

#[test]
fn a_retry_is_granted_every_file_the_hosts_judged_run_saw_its_test_fail_in() {
    use crate::v2::write::test_baseline_run_base::{HostRunVerdict, Tree, cache};
    let w = package_world();
    let extra = "crates/shared/src/fixtures.rs";
    std::fs::write(w.root().join(extra), "//\n").unwrap();
    let (_, first) = first_round(&w, "high");
    let mut judge = round_verdict(
        &first,
        "verification-wave-review-verify-residual-4",
        1,
        &[STALE],
        &[],
    );
    judge.result.data["outcomes"][0]["result"]["data"]["judged_commit"] = json!("judged");
    w.save(&judge);
    cache(
        &w.store,
        Tree::Judged,
        &HostRunVerdict {
            command: LIB.into(),
            commit: "judged".into(),
            exit_code: Some(101),
            failing_tests: vec![STALE.into()],
            failure_files: [(STALE.to_string(), vec![extra.to_string()])].into(),
            ..Default::default()
        },
    );
    let retry = second(&w).rounds[0].clone();
    assert_eq!(ids(&retry.files), [extra, STORE, TESTS]);
}
