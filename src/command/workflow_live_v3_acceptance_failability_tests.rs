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

fn round(n: u64) -> archon_workflow::WorkflowV2CallExecution {
    let (options, _) = archon_workflow::v2::script::parse_script_options(&serde_json::json!({
        "tool": archon_workflow::v2::acceptance_stage::ACCEPTANCE_STAGE_TOOL,
        "round": n, "maxRounds": 3, "checkIds": [],
    }))
    .unwrap();
    archon_workflow::WorkflowV2CallExecution {
        call: archon_workflow::WorkflowV2HostCall {
            id: format!("acceptance-contract-run-{n}"),
            method: archon_workflow::WorkflowV2HostMethod::Tool,
            write_mode: None,
            options,
        },
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    }
}

/// With no pre-implementation tree a passing check never passes the round;
/// the rounds repeat, the run PAUSES (never fails), and the pause evidence
/// tells the operator how to give the run a base and resume.
#[tokio::test]
async fn a_passing_check_with_no_pre_implementation_tree_pauses_naming_the_way_out() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let (_, record) = stage(&run, &passes_unchanged()).await;
    assert!(record.blocks_completion(), "{record:?}");
    let error = (record.operational_errors.iter())
        .find(|error| error.contains("'AC-F-001' can fail") && error.contains("no base commit"))
        .unwrap_or_else(|| panic!("{record:?}"));
    for way_out in [
        "git checkout",
        "repository.lock",
        "base_commit",
        "then resume",
    ] {
        assert!(error.contains(way_out), "{way_out}: {error}");
    }
    let client = passes_unchanged();
    let mut paused = None;
    for n in 2..=6 {
        let outcome = super::super::run_acceptance_stage(
            &run.runtime,
            &round(n),
            &run.store,
            &run.run_id,
            run.store.load_state(&run.run_id).unwrap().generation,
            Some(&run.universe),
            Some(&client),
        )
        .await;
        match outcome {
            Ok(_) => continue,
            Err(archon_workflow::WorkflowError::ControlPaused(message)) => {
                paused = Some(message);
                break;
            }
            Err(other) => panic!("a missing base pauses, never fails: {other:?}"),
        }
    }
    let message = paused.expect("the repeated round pauses the run");
    assert!(message.contains("resume"), "{message}");
    let events = std::fs::read_to_string(run.store.events_path(&run.run_id)).unwrap();
    let pause = (events.lines())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["detail"]["event"] == "acceptance_stall_pause")
        .expect("pause evidence");
    let evidence = pause["detail"]["operational_errors"].to_string();
    assert!(
        evidence.contains("repository.lock") && evidence.contains("git checkout"),
        "{evidence}"
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
