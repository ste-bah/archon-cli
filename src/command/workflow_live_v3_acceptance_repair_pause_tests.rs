//! Issue 336: a repair on the last round whose republished set is left with
//! a journal no read can settle pauses the run; it never ends the round as a
//! failed repair, which on the last round would fail the run.

use super::super::repair_tests::{Run, record_baseline, run_fixture_with};
use super::super::*;
use crate::command::workflow_task_set::executability::tests::{CRASHING, FIXED};
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use archon_workflow::{WorkflowV2HostCall, WorkflowV2HostMethod};

fn pin_path(run: &Run) -> std::path::PathBuf {
    crate::command::workflow_task_set::acceptance_pin_path(run.set.project.path(), &run.set.tasks)
}

/// The last round the stage may run: a failed repair here is final.
fn last_round() -> WorkflowV2CallExecution {
    let (options, _) = archon_workflow::v2::script::parse_script_options(&serde_json::json!({
        "tool": ACCEPTANCE_STAGE_TOOL, "round": 3, "maxRounds": 3, "checkIds": [],
    }))
    .unwrap();
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "acceptance-contract-run-3".into(),
            method: WorkflowV2HostMethod::Tool,
            write_mode: None,
            options,
        },
        input: serde_json::json!({}),
        depends_on: Vec::new(),
    }
}

async fn run_last_round(
    run: &Run,
    client: &ScriptedAuthorJudge,
) -> WorkflowResult<WorkflowV2Result> {
    run_acceptance_stage(
        &run.runtime,
        &last_round(),
        &run.store,
        &run.run_id,
        run.store.load_state(&run.run_id).unwrap().generation,
        Some(&run.universe),
        Some(client),
    )
    .await
}

fn assert_paused(run: &Run, outcome: WorkflowResult<WorkflowV2Result>) {
    let error = match outcome {
        Ok(result) => panic!(
            "the last round ended {:?} instead of pausing the run: {}",
            result.status, result.summary
        ),
        Err(error) => error,
    };
    let WorkflowError::ControlPaused(evidence) = error else {
        panic!("the last round failed instead of pausing: {error}");
    };
    let journal = pin_path(run).with_extension("publish-journal");
    for needed in [
        journal.display().to_string(),
        "state: committed".into(),
        "Operator remedy".into(),
        "archon workflow resume --live --yes".into(),
    ] {
        assert!(evidence.contains(&needed), "{needed} missing: {evidence}");
    }
    assert!(journal.exists(), "the journal's decision was lost");
}

/// A refuted check the judge accepts on re-authoring (`repair::apply`).
fn refuted() -> (Run, ScriptedAuthorJudge) {
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", "test -f missing", false),
    ]);
    record_baseline(&run);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f present && test -s present"),
        |_, _| true,
    );
    (run, client)
}

#[tokio::test]
async fn a_refuted_check_repaired_on_the_last_round_pauses_on_an_unsettled_journal() {
    let (run, client) = refuted();
    let _fix = crate::command::workflow_task_set::stick_next_commit(&pin_path(&run));
    assert_paused(&run, run_last_round(&run, &client).await);
}

#[tokio::test]
async fn a_crashed_check_repaired_on_the_last_round_pauses_on_an_unsettled_journal() {
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", CRASHING, true),
    ]);
    record_baseline(&run);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, FIXED), |_, _| true);
    let _fix = crate::command::workflow_task_set::stick_next_commit(&pin_path(&run));
    assert_paused(&run, run_last_round(&run, &client).await);
}

#[tokio::test]
async fn a_last_round_paused_on_an_unsettled_journal_is_accepted_once_the_operator_fixes_it() {
    let (run, client) = refuted();
    let fix = crate::command::workflow_task_set::stick_next_commit(&pin_path(&run));
    assert_paused(&run, run_last_round(&run, &client).await);
    fix();
    let result = run_last_round(&run, &client)
        .await
        .expect("the resumed round settles the journal and runs");
    assert!(
        !pin_path(&run).with_extension("publish-journal").exists(),
        "the resumed read settled the journal"
    );
    assert_eq!(
        result.status,
        WorkflowV2Status::Accepted,
        "{}",
        result.summary
    );
    let (record, _) =
        archon_workflow::v2::acceptance_stage::latest_round_record(&run.store.run_dir(&run.run_id))
            .unwrap()
            .unwrap();
    assert!(record.failing_checks().is_empty(), "{:?}", record.checks);
    assert!(
        record.operational_errors.is_empty(),
        "{:?}",
        record.operational_errors
    );
}
