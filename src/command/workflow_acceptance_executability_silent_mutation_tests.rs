#![allow(non_snake_case)]
use super::super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[test]
fn P1_12_baseline_timeout_text_unchanged() {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    assert!(super::super::settle_timed_out(&probe, &trees.base, &contract, "AC-X-001").is_none());
    let text = super::super::settle_timed_out(&probe, &trees.base, &contract, "AC-X-001").unwrap();
    assert!(text.contains("it made no progress on the pre-implementation tree"));
    assert!(text.contains("Repair or replace it so that it ends by its own assertion"));
}
#[test]
fn P1_13_mutation_timeout_with_store() {
    timeout(true);
}
#[test]
fn P1_14_mutation_timeout_without_store() {
    timeout(false);
}
fn timeout(memo: bool) {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let mut probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    probe.memo = memo;
    assert_eq!(probe.result_store().is_some(), memo);
    let contract = trees.contract();
    let moved = ["data/state.txt".into()];
    assert!(
        settle_mutation_timeout(&probe, &trees.base, &contract, "AC-X-001", &moved, "states")
            .is_none()
    );
    // A fresh wrapper nonce never enters the identity; only original text,
    // commit, id, window and the mutation phase key the durable strike.
    let _fresh_nonce = super::super::super::mutation::Markers::new();
    let text =
        settle_mutation_timeout(&probe, &trees.base, &contract, "AC-X-001", &moved, "states")
            .unwrap();
    assert!(text.contains("with data/state.txt moved aside it made no progress"));
    assert!(text.contains("it must fail, not wait, when an input is missing"));
    assert!(!text.contains("on the pre-implementation tree"));
}
#[test]
fn P1_15_mutation_verdict_clears_timeout_strike() {
    let trees = trees(&[(
        "AC-X-001",
        "test -f data/state.txt",
        TrustedCwd::ProjectRoot,
    )]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let moved = ["data/state.txt".into()];
    assert!(
        settle_mutation_timeout(&probe, &trees.base, &contract, "AC-X-001", &moved, "").is_none()
    );
    ran_mutation(&probe, &trees.base, &contract, "AC-X-001");
    assert!(
        settle_mutation_timeout(&probe, &trees.base, &contract, "AC-X-001", &moved, "").is_none()
    );
}

#[test]
fn timeout_without_a_moved_input_has_no_empty_splice() {
    let trees = trees(&[("AC-X-001", "true", TrustedCwd::ProjectRoot)]);
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let contract = trees.contract();
    let _ = settle_mutation_timeout(&probe, &trees.base, &contract, "AC-X-001", &[], "");
    let text =
        settle_mutation_timeout(&probe, &trees.base, &contract, "AC-X-001", &[], "").unwrap();
    assert!(!text.contains("with  moved aside"));
    HostProbe::assert_p1_readers(&text, false);
}
