use super::super::test_baseline::{
    BaselineObligation, BranchBaseline, CommandBaseline, IgnoredFailure, RoutedFailure,
    SCHEMA_VERSION,
};
use super::preamble;

fn record() -> BranchBaseline {
    BranchBaseline {
        schema_version: SCHEMA_VERSION,
        stage_id: "agents-3".into(),
        branch_id: "agents-3-impl".into(),
        base_commit: "0123456789abcdef0123".into(),
        canonical_task_ids: vec!["TASK-A".into()],
        commands: vec![CommandBaseline {
            command: "cargo test -p engine grant".into(),
            base_commit: "0123456789abcdef0123".into(),
            exit_code: Some(101),
            timed_out: false,
            duration_ms: 12,
            failing_tests: vec!["grant::tests::mine".into(), "plan::tests::theirs".into()],
            tail: Vec::new(),
            error: None,
            cached: false,
        }],
        obligations: vec![BaselineObligation {
            test_id: Some("grant::tests::mine".into()),
            file: Some("crates/engine/src/grant_tests.rs".into()),
            command: "cargo test -p engine grant".into(),
        }],
        routed: vec![RoutedFailure {
            test_id: "plan::tests::theirs".into(),
            file: "crates/engine/src/plan/mod.rs".into(),
            owner_task: "TASK-B".into(),
            command: "cargo test -p engine grant".into(),
        }],
        ignored: vec![IgnoredFailure {
            test_id: "gate::frozen".into(),
            file: "crates/engine/src/gate.rs".into(),
            reason: "its file is forbidden to this task and no other task declares it".into(),
        }],
        inherited: vec![BaselineObligation {
            test_id: Some("grant::tests::routed_in".into()),
            file: Some("crates/engine/src/grant.rs".into()),
            command: "cargo test -p engine".into(),
        }],
    }
}

#[test]
fn the_section_names_every_bucket_by_test_id_and_states_the_rule() {
    let text = preamble(&record());
    assert!(text.starts_with("\nBaseline tests (the host ran your declared focused test commands on the base commit 0123456789ab, in this worktree, before you started):\n"), "{text}");
    assert!(text.contains("- Tests already failing on the base commit within your declared filter: grant::tests::mine (crates/engine/src/grant_tests.rs) — these are yours to make pass; their files are in your scope.\n"), "{text}");
    assert!(text.contains("- Tests already failing on the base commit in files you declare, found by another task's filter: grant::tests::routed_in (crates/engine/src/grant.rs) — these are yours to make pass too.\n"), "{text}");
    assert!(text.contains("- Tests already failing on the base commit within your declared filter but owned by another task: plan::tests::theirs — owned by TASK-B, ignore. Do not edit their files; they are routed to their owner.\n"), "{text}");
    assert!(text.contains("- Tests already failing on the base commit you must leave alone: gate::frozen (crates/engine/src/gate.rs; its file is forbidden to this task and no other task declares it).\n"), "{text}");
    assert!(text.ends_with("Your task is not accepted while any test in your declared filter fails, except the ones listed above as owned by another task or to leave alone; \"pre-existing\" is not an acceptable reason, and neither is disabling or deleting the test.\n"), "{text}");
}

#[test]
fn a_green_baseline_says_so_and_an_unbaselined_command_is_named_with_its_reason() {
    let mut green = record();
    green.obligations.clear();
    green.routed.clear();
    green.ignored.clear();
    green.inherited.clear();
    green.commands[0].exit_code = Some(0);
    green.commands[0].failing_tests.clear();
    let text = preamble(&green);
    assert!(text.contains("- Every test in your declared filter passes on the base commit; any red test after your change is yours.\n"), "{text}");

    let mut unknown = green.clone();
    unknown.commands[0].exit_code = None;
    unknown.commands[0].timed_out = true;
    unknown.commands[0].error = Some("baseline command timed out after 10s".into());
    let text = preamble(&unknown);
    assert!(text.contains("- Declared commands the host could not baseline: `cargo test -p engine grant` (baseline command timed out after 10s)"), "{text}");
    assert!(
        !text.contains("Every test in your declared filter passes"),
        "{text}"
    );
}

#[test]
fn nothing_declared_means_no_section() {
    let mut empty = record();
    empty.commands.clear();
    empty.obligations.clear();
    empty.routed.clear();
    empty.ignored.clear();
    empty.inherited.clear();
    assert_eq!(preamble(&empty), "");
}
