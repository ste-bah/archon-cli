//! A round whose check crashed in its own code repairs the contract in-round
//! and never hands the crash to the implementing tasks.

use super::repair_tests::{run_fixture_with, stage};
use super::*;
use crate::command::workflow_task_set::executability::tests::{CRASHING, FIXED};
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::republish::test_fixture::assert_only_named_entries_changed;
use archon_workflow::v2::acceptance_stage::REPAIR_TRIGGER_SCRIPT_DEFECT;

const SIGNAL: &str = "lane() missing 1 required positional argument: 'd'";

fn check<'a>(record: &'a AcceptanceRoundRecordV1, id: &str) -> &'a AcceptanceCheckRecordV1 {
    record.checks.iter().find(|c| c.check_id == id).unwrap()
}

#[tokio::test]
async fn a_check_crashing_in_its_own_code_is_repaired_republished_and_rerun_in_the_round() {
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", CRASHING, true),
    ]);
    let before = run.set.contract_bytes();
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, FIXED), |_, _| true);
    let (result, record) = stage(&run, &client).await;
    assert_eq!(
        record.contract_repairs.len(),
        1,
        "{:?}",
        record.contract_repairs
    );
    let repair = &record.contract_repairs[0];
    assert!(repair.repaired, "{repair:?}");
    assert_eq!(repair.trigger, REPAIR_TRIGGER_SCRIPT_DEFECT);
    assert_eq!(repair.check_ids, vec!["AC-F-002".to_string()]);
    assert!(record.failing_checks().is_empty(), "{:?}", record.checks);
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001", "AC-F-002"]);
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    let named = ["AC-F-002".to_string()].into_iter().collect();
    assert_only_named_entries_changed(&before, &run.set.contract_bytes(), &named);
    archon_workflow::task_skeleton::validate_full_chain(&run.set.tasks, &run.set.pin())
        .expect("the republished chain verifies");
    assert!(
        client.prompts.lock().unwrap()[0].contains(SIGNAL),
        "the author was shown the round's crash"
    );
    let evidence = round_dir(&run.store.run_dir(&run.run_id), 1)
        .join("attempt-01/script-defect/AC-F-002.stderr");
    assert!(std::fs::read_to_string(evidence).unwrap().contains(SIGNAL));
}

#[tokio::test]
async fn a_repaired_check_that_then_fails_its_assertion_goes_to_its_tasks() {
    let crashing = CRASHING.replace("'present'", "'missing'");
    let fixed = FIXED.replace("'present'", "'missing'");
    let run = run_fixture_with(&[("AC-F-001", &crashing, true)]);
    let client =
        ScriptedAuthorJudge::new(move |entry, _| command_entry(entry, &fixed), |_, _| true);
    let (_, record) = stage(&run, &client).await;
    assert!(record.contract_repairs[0].repaired);
    let failing = check(&record, "AC-F-001");
    assert_eq!(failing.status, AcceptanceCheckStatus::Failed);
    assert!(!failing.contract_defect);
    assert_eq!(failing.owning_tasks, vec!["TASK-F-001"]);
    assert!(
        failing.stderr_tail.contains("deliverable missing"),
        "{failing:?}"
    );
    assert!(record.has_remediable_failures());
}

#[tokio::test]
async fn a_crash_the_repair_cannot_fix_is_a_contract_defect_no_task_owns() {
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", CRASHING, true),
    ]);
    let before = run.set.contract_bytes();
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("{CRASHING}# {attempt}\n")),
        |_, _| true,
    );
    let (result, record) = stage(&run, &client).await;
    assert!(!record.contract_repairs[0].repaired);
    assert_eq!(
        record.contract_repairs[0].trigger,
        REPAIR_TRIGGER_SCRIPT_DEFECT
    );
    let defect = check(&record, "AC-F-002");
    assert!(defect.contract_defect, "{defect:?}");
    assert!(defect.owning_tasks.is_empty(), "never handed to a task");
    assert_eq!(
        defect.exit_code,
        Some(1),
        "the crash's own evidence is kept"
    );
    assert!(defect.stderr_tail.contains(SIGNAL), "{defect:?}");
    let text = defect.operational_error.as_deref().unwrap();
    assert!(text.contains(SIGNAL), "{text}");
    assert!(
        text.contains("freeze-acceptance --reauthor AC-F-002"),
        "{text}"
    );
    assert!(!record.has_remediable_failures());
    assert!(record.final_round);
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(run.set.contract_bytes(), before, "nothing was published");
}

#[tokio::test]
async fn a_check_failing_on_its_own_assertion_is_never_reauthored() {
    let fixed = FIXED.replace("'present'", "'missing'");
    let run = run_fixture_with(&[("AC-F-001", &fixed, true)]);
    let before = run.set.contract_bytes();
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "true"), |_, _| true);
    let (_, record) = stage(&run, &client).await;
    assert!(record.contract_repairs.is_empty());
    assert_eq!(client.authored(), 0);
    let failing = check(&record, "AC-F-001");
    assert_eq!(failing.owning_tasks, vec!["TASK-F-001"]);
    assert!(!failing.contract_defect);
    assert_eq!(run.set.contract_bytes(), before);
}
