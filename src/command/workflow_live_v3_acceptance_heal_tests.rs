//! Issue 262, round 8 of the review (P1): damaged or unreadable acceptance
//! history never fails the run and never leaves it Running with no
//! executor. A damaged record is quarantined and the loop goes on from its
//! ledger copy; without one, or on an I/O error, the run PAUSES with the
//! reason and the resume command, and a resume goes on.

use std::os::unix::fs::PermissionsExt;

use super::*;
use archon_workflow::v2::acceptance_stage::{attempt_file_name, round_dir};

fn events(fixture: &Fixture) -> Vec<serde_json::Value> {
    std::fs::read_to_string(fixture.store.events_path(&fixture.run_id))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn has_event(fixture: &Fixture, name: &str) -> bool {
    (events(fixture).iter()).any(|event| event["detail"]["event"] == name)
}

fn resume(fixture: &Fixture) {
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
}

fn status(fixture: &Fixture) -> RunStatus {
    fixture.store.load_state(&fixture.run_id).unwrap().status
}

fn run_dir(fixture: &Fixture) -> std::path::PathBuf {
    fixture.store.run_dir(&fixture.run_id)
}

fn first_record(fixture: &Fixture) -> std::path::PathBuf {
    round_dir(&run_dir(fixture), 1).join(attempt_file_name(1))
}

fn assert_paused(fixture: &Fixture, outcome: &WorkflowResult<WorkflowV2Result>) -> String {
    let Err(WorkflowError::ControlPaused(message)) = outcome else {
        panic!("damaged history pauses the run, never fails it: {outcome:?}");
    };
    assert_eq!(
        status(fixture),
        RunStatus::Paused,
        "never Running with no executor"
    );
    assert!(
        message.contains(&fixture.run_id),
        "names the resume: {message}"
    );
    message.clone()
}

/// The damaged record's state is in the ledger's copy: it is quarantined,
/// the loop goes on, and the rebuild is exact -- round 2 revisits round
/// 1's failing set (escalates) and round 3's second revisit pauses.
#[tokio::test]
async fn a_corrupt_record_with_its_ledger_copy_is_quarantined_and_the_loop_goes_on() {
    let fixture = fixture_with(true, "test -f missing");
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let damaged = first_record(&fixture);
    std::fs::write(&damaged, "{\"round\":").unwrap();

    let second = run(&fixture, &execution(2, 3, &[])).await;

    let second = second.expect("a corrupt record never fails the round");
    assert_eq!(second.data["escalate"], true, "{second:#?}");
    assert_ne!(status(&fixture), RunStatus::Paused);
    assert!(!damaged.exists(), "moved aside");
    let quarantine = damaged.parent().unwrap().join("quarantine");
    let kept = (std::fs::read_dir(&quarantine).unwrap())
        .map(|entry| std::fs::read(entry.unwrap().path()).unwrap())
        .any(|bytes| bytes == b"{\"round\":");
    assert!(kept, "the damaged bytes are kept, never deleted");
    assert!(has_event(&fixture, "acceptance_record_quarantined"));
    let third = run(&fixture, &execution(3, 3, &[])).await;
    assert_paused(&fixture, &third);
}

/// No copy of the damaged record's state: the record is quarantined and the
/// run pauses once with the reason; the resume runs the round again on the
/// remaining evidence and goes on.
#[tokio::test]
async fn a_corrupt_record_with_no_copy_pauses_once_and_a_resume_goes_on() {
    let fixture = fixture_with(true, "test -f missing");
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    std::fs::remove_file(run_dir(&fixture).join("v2/acceptance/progress-ledger.json")).unwrap();
    std::fs::write(first_record(&fixture), "{\"round\":").unwrap();

    let second = run(&fixture, &execution(2, 3, &[])).await;

    let message = assert_paused(&fixture, &second);
    assert!(message.contains("quarantine"), "{message}");
    assert!(has_event(&fixture, "acceptance_record_quarantined"));
    assert!(has_event(&fixture, "acceptance_history_pause"));
    resume(&fixture);
    let again = (run(&fixture, &execution(2, 3, &[])).await).expect("a resume goes on");
    assert_eq!(status(&fixture), RunStatus::Running, "{again:#?}");
    let pauses = (events(&fixture).iter())
        .filter(|event| event["detail"]["event"] == "acceptance_history_pause")
        .count();
    assert_eq!(pauses, 1, "paused once, at discovery");
}

/// An order log the file system will not hand over is an I/O fault: the
/// run pauses (never fails), and once it reads again a resume goes on.
#[tokio::test]
async fn an_unreadable_order_log_pauses_and_a_resume_goes_on() {
    let fixture = fixture_with(true, "test -f missing");
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let log = run_dir(&fixture).join("v2/acceptance/recording-order.log");
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o000)).unwrap();

    let second = run(&fixture, &execution(2, 3, &[])).await;

    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_paused(&fixture, &second);
    resume(&fixture);
    run(&fixture, &execution(2, 3, &[]))
        .await
        .expect("a resume goes on");
}

/// A record the file system will not hand over is an I/O fault, not
/// damage: the run pauses, the record stays where it is, and a resume goes
/// on once it reads.
#[tokio::test]
async fn an_io_error_reading_a_record_pauses_and_a_resume_goes_on() {
    let fixture = fixture_with(true, "test -f missing");
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let record = first_record(&fixture);
    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o000)).unwrap();

    let second = run(&fixture, &execution(2, 3, &[])).await;

    std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_paused(&fixture, &second);
    assert!(record.exists(), "an unreadable record is never quarantined");
    resume(&fixture);
    let again = (run(&fixture, &execution(2, 3, &[])).await).expect("a resume goes on");
    assert_eq!(
        again.data["escalate"], true,
        "the record counts again: {again:#?}"
    );
}

struct NoLlm;

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for NoLlm {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        panic!("unexpected LLM request")
    }
}

/// The review's reproduction through the real script host: corrupt
/// history with no copy pauses the run, never a Failed summary over a run
/// left Running.
#[tokio::test]
async fn the_script_host_pauses_on_corrupt_history() {
    use super::super::super::{LiveV2AgentClient, WorkflowV2ScriptRunner};
    let fixture = fixture_with(true, "test -f missing");
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    std::fs::remove_file(run_dir(&fixture).join("v2/acceptance/progress-ledger.json")).unwrap();
    std::fs::write(first_record(&fixture), "{\"round\":").unwrap();
    let (ui_sink, _receiver) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        std::sync::Arc::new(NoLlm),
        ui_sink,
        vec![],
        fixture.run_id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "corrupt history".into(),
        fixture.runtime.clone(),
        archon_workflow::WorkflowV2AgentAdapter::new(),
        client,
        archon_workflow::WorkflowV2ResultStore::new(run_dir(&fixture).join("v2")),
        fixture.store.clone(),
        fixture.run_id.clone(),
        true,
        Some(fixture.universe.clone()),
        None,
    );
    let script = r#"async function workflow(w) { return await w.tool("acceptance-contract-run-2", {tool:"acceptance-contract-run",round:2,maxRounds:3}); }"#;

    let result = runner.run(script).await;

    assert!(
        matches!(result, Err(WorkflowError::ControlPaused(_))),
        "a pause, never a Failed summary: {:?}",
        result.as_ref().map(|summary| &summary.status)
    );
    assert_eq!(status(&fixture), RunStatus::Paused);
}

/// Round 9 (P2): the process died after quarantining a record of unknown
/// state and before it paused. The next execution still pauses on the
/// loss (never goes on silently), and the resume after that goes on.
#[tokio::test]
async fn a_loss_quarantined_before_a_crash_still_pauses_the_next_execution() {
    use archon_workflow::v2::acceptance_stage::progress::ProgressLedger;
    let fixture = fixture_with(true, "test -f missing");
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    std::fs::remove_file(run_dir(&fixture).join("v2/acceptance/progress-ledger.json")).unwrap();
    std::fs::write(first_record(&fixture), "{\"round\":").unwrap();
    // Moved to quarantine; the process dies before it pauses.
    let healed = ProgressLedger::load_healing(&run_dir(&fixture)).unwrap();
    assert_eq!(healed.unknown().len(), 1, "{healed:?}");

    let second = run(&fixture, &execution(2, 3, &[])).await;

    let message = assert_paused(&fixture, &second);
    assert!(message.contains("quarantine"), "{message}");
    resume(&fixture);
    run(&fixture, &execution(2, 3, &[]))
        .await
        .expect("acknowledged by the pause: a resume goes on");
}

/// Issue 317: a record of unknown state was quarantined (the process died
/// before it paused), then its quarantine evidence was damaged too, and the
/// saved ledger is a legacy one. The lost record is never ignored and the
/// legacy ledger never stands in for it: the round pauses on the loss with
/// the reason, the damaged evidence bytes are kept, and the resume goes on.
#[tokio::test]
async fn damaged_quarantine_evidence_pauses_on_the_lost_record() {
    use archon_workflow::v2::acceptance_stage::progress::ProgressLedger;
    let fixture = fixture_with(true, "test -f missing");
    run(&fixture, &execution(1, 3, &[])).await.unwrap();
    let ledger = run_dir(&fixture).join("v2/acceptance/progress-ledger.json");
    std::fs::write(&ledger, r#"{"seen":[["AC-OTHER"]],"revisits":1}"#).unwrap();
    std::fs::write(first_record(&fixture), "{\"round\":").unwrap();
    let healed = ProgressLedger::load_healing(&run_dir(&fixture)).unwrap();
    let moved = run_dir(&fixture).join(&healed.quarantined[0].quarantined);
    let stem = moved.file_name().unwrap().to_str().unwrap();
    let evidence = moved.with_file_name(stem.replace(".damaged", ".evidence.json"));
    std::fs::write(&evidence, "{\"event\":").unwrap();

    let second = run(&fixture, &execution(2, 3, &[])).await;

    let message = assert_paused(&fixture, &second);
    assert!(message.contains("evidence"), "{message}");
    assert!(has_event(&fixture, "acceptance_history_pause"));
    let kept = (std::fs::read_dir(moved.parent().unwrap()).unwrap())
        .any(|entry| std::fs::read(entry.unwrap().path()).unwrap() == b"{\"event\":");
    assert!(kept, "the damaged evidence bytes are kept");
    resume(&fixture);
    run(&fixture, &execution(2, 3, &[]))
        .await
        .expect("acknowledged by the pause: a resume goes on");
}
