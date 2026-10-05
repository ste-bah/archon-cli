//! Issue 219 item 5 in a real round: a frozen check that passes is counted
//! only once it is proven able to fail on the run's pre-implementation tree.

use super::super::repair_tests::{record_baseline, run_fixture_with, stage};
use super::REPAIR_TRIGGER_CANNOT_FAIL;
use crate::command::workflow_task_set::executability::tests::FIXED;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};

/// Passes on any tree: it reads nothing (a replay compared with itself).
const VACUOUS: &str = "python3 -c 'import sys; sys.exit(0)'";

fn passes_unchanged() -> ScriptedAuthorJudge {
    ScriptedAuthorJudge::new(|entry, _| command_entry(entry, VACUOUS), |_, _| true)
}

#[tokio::test]
async fn a_frozen_check_that_passes_on_the_base_commit_is_repaired_before_it_counts() {
    let run = run_fixture_with(&[("AC-F-001", VACUOUS, true)]);
    record_baseline(&run);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, FIXED), |_, _| true);
    let (_, record) = stage(&run, &client).await;
    let repair = (record.contract_repairs.iter())
        .find(|repair| repair.trigger == REPAIR_TRIGGER_CANNOT_FAIL)
        .unwrap_or_else(|| panic!("the vacuous pass was counted: {record:?}"));
    assert!(repair.repaired, "{repair:?}");
    assert_eq!(repair.check_ids, vec!["AC-F-001"]);
    let prompts = client.prompts.lock().unwrap().clone();
    assert!(
        prompts[0].contains("passed on the pre-implementation tree at"),
        "{}",
        prompts[0]
    );
    // The repaired check fails where nothing was built and passes here.
    assert!(!record.blocks_completion(), "{record:?}");
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001"]);
}

#[tokio::test]
async fn a_check_that_cannot_fail_and_cannot_be_repaired_blocks_the_round() {
    let run = run_fixture_with(&[("AC-F-001", VACUOUS, true)]);
    record_baseline(&run);
    let (_, record) = stage(&run, &passes_unchanged()).await;
    assert!(record.blocks_completion(), "{record:?}");
    assert!(record.passed_check_ids().is_empty(), "{record:?}");
    let check = &record.checks[0];
    assert!(check.contract_defect, "{check:?}");
    let defect = check.operational_error.as_deref().unwrap_or_default();
    assert!(
        defect.contains("passes whether or not its criterion holds"),
        "{defect}"
    );
}

#[tokio::test]
async fn a_passing_check_with_no_pre_implementation_tree_never_passes_the_round() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let (_, record) = stage(&run, &passes_unchanged()).await;
    assert!(record.blocks_completion(), "{record:?}");
    assert!(
        (record.operational_errors.iter())
            .any(|error| error.contains("'AC-F-001' can fail") && error.contains("no base commit")),
        "{record:?}"
    );
}

#[tokio::test]
async fn a_proven_check_is_never_probed_again_for_the_same_base() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    record_baseline(&run);
    let (_, first) = stage(&run, &passes_unchanged()).await;
    assert!(!first.blocks_completion(), "{first:?}");
    // Without its git history the base can no longer be probed: a second
    // round that passes used the recorded proof.
    let project = run.set.project.path();
    std::fs::rename(project.join(".git"), project.join("git-moved")).unwrap();
    let (_, second) = stage(&run, &passes_unchanged()).await;
    assert!(!second.blocks_completion(), "{second:?}");
    assert_eq!(second.passed_check_ids(), vec!["AC-F-001"]);
}
