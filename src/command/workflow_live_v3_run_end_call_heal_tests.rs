//! Issue 313: the acceptance call's own record can be damaged by the time
//! the run ends. Not JSON and the wrong shape are the same damage: never an
//! error that fails the run, never "the stage recorded no round", and never
//! a verdict guessed from a round record (round 2: the newest round of the
//! call can be stale). The damage is quarantined and the run pauses; the
//! resume runs the stage again and heals it.

use super::*;
use archon_workflow::{LifecycleAction, LifecycleController};

/// Not JSON, and JSON carrying every key the store looks for, wrongly typed.
const DAMAGE: [&str; 2] = [
    "{\"call\":",
    r#"{"call":{"id":"x"},"attempt":"one","input_hash":1,"status":"accepted","result":{}}"#,
];

fn damage_call(fixture: &Fixture, summary: &WorkflowV2ScriptSummary, damage: &[u8]) {
    std::fs::write(fixture.v2_store.result_path(&summary.calls[0].id), damage).unwrap();
}

fn status(fixture: &Fixture) -> RunStatus {
    fixture.store.load_state(&fixture.run_id).unwrap().status
}

fn paused_message(error: &anyhow::Error) -> Option<String> {
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WorkflowError>())
    {
        Some(WorkflowError::ControlPaused(message)) => Some(message.clone()),
        _ => None,
    }
}

/// The run end: the damaged record is quarantined with an event and the
/// run pauses naming it, never Completed on a guess. The resume runs the
/// stage again and the run ends on the new round.
#[tokio::test]
async fn a_damaged_acceptance_call_record_pauses_with_evidence_and_the_resume_heals() {
    for damage in DAMAGE {
        let fixture = fixture();
        let summary = in_run_round(&fixture).await;
        damage_call(&fixture, &summary, damage.as_bytes());

        let error = try_finalize_with(&fixture, &scripted(&fixture, 0), summary)
            .await
            .expect_err("a damaged call record pauses the run");

        let message = paused_message(&error).unwrap_or_else(|| panic!("{damage}: {error:#}"));
        assert!(message.contains("acceptance call record"), "{message}");
        assert_eq!(status(&fixture), RunStatus::Paused, "{damage}");
        assert!(
            labels(&fixture)
                .iter()
                .any(|l| l == "acceptance_call_record_quarantined"),
            "{damage}: {:?}",
            labels(&fixture)
        );
        LifecycleController::new(fixture.store.clone())
            .apply(&fixture.run_id, LifecycleAction::Resume)
            .unwrap();
        let summary = in_run_round(&fixture).await;
        let finalized = finalize_with(&fixture, &scripted(&fixture, 0), summary).await;
        assert_eq!(finalized.status, WorkflowV2Status::Accepted, "{damage}");
        assert_eq!(status(&fixture), RunStatus::Completed, "{damage}");
        let gate = record(&fixture)
            .acceptance_gate
            .expect("the new round's gate");
        assert_eq!(gate.record_path, "v2/acceptance/round-01/attempt-02.json");
    }
}

/// Round 2 (review repro): round 1 passed as attempt-01; a later execution
/// of the same call failed before writing a round, and its record is cut in
/// half. The call's newest round is stale: no Accepted record and no clean
/// gate may come from it. The run pauses.
#[tokio::test]
async fn a_damaged_failed_reexecution_is_never_judged_on_a_stale_round() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let id = &summary.calls[0].id;
    let mut failed = fixture.v2_store.load_call_record(id).unwrap().unwrap();
    failed.attempt += 1;
    failed.status = WorkflowV2Status::Failed;
    failed.result = archon_workflow::v2::host_fault::v2_result_for_call_error(
        id,
        &WorkflowError::HostOperational("stage errored before its round".into()),
    );
    let bytes = serde_json::to_vec(&failed).unwrap();
    damage_call(&fixture, &summary, &bytes[..bytes.len() / 2]);

    let call = super::super::call::acceptance_call_record(
        &fixture.store,
        &fixture.run_id,
        &fixture.v2_store,
        id_call(&summary),
    );

    assert!(
        matches!(&call, Err(WorkflowError::ControlPaused(_))),
        "a stale round must not stand in for the damaged record: {call:?}"
    );
    assert_eq!(status(&fixture), RunStatus::Paused);
}

fn id_call(summary: &WorkflowV2ScriptSummary) -> &archon_workflow::WorkflowV2HostCall {
    &summary.calls[0]
}

/// Inside the runner, through the real terminal rule: with the whole record
/// the rule judges the script's accounting; with the record damaged the run
/// pauses before any verdict, never errors and never judges a guess.
#[tokio::test]
async fn the_authored_outcome_pauses_on_a_damaged_acceptance_call() {
    let accounting = serde_json::json!({
        "accepted": ["TASK-H-001"], "blocked": [], "adversarial_findings": [],
        "uncovered_requirements": [],
        "review_remediation": { "resolved": [], "unresolved": [], "unassigned": [] },
    })
    .to_string();
    let outcome = |fixture: &Fixture, mut summary: WorkflowV2ScriptSummary| {
        summary.script_result = Some(accounting.clone());
        apply_authored_run_outcome(
            &fixture.store,
            &fixture.run_id,
            &fixture.v2_store,
            Some(&fixture.universe),
            Some(fixture.repo.path()),
            true,
            summary,
        )
    };
    let judged = |fixture: &Fixture| {
        std::fs::read_to_string(fixture.store.events_path(&fixture.run_id))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|event| event["detail"]["event"] == "authored_run_outcome")
            .map(|event| event["detail"]["from_accounting"].clone())
    };
    let whole = fixture();
    let summary = in_run_round(&whole).await;
    outcome(&whole, summary).expect("a whole record is judged");
    assert_eq!(
        judged(&whole),
        Some(serde_json::json!(true)),
        "the rule ran"
    );
    for damage in DAMAGE {
        let fixture = fixture();
        let summary = in_run_round(&fixture).await;
        damage_call(&fixture, &summary, damage.as_bytes());

        let got = outcome(&fixture, summary);

        assert!(
            matches!(&got, Err(WorkflowError::ControlPaused(_))),
            "{damage}: {got:?}"
        );
        assert_eq!(status(&fixture), RunStatus::Paused, "{damage}");
        assert_eq!(judged(&fixture), None, "{damage}: no verdict on damage");
    }
}

/// The file system will not hand the call record over (here: a directory
/// stands in the slot): the run pauses naming it, never Failed.
#[tokio::test]
async fn an_unreadable_acceptance_call_record_pauses_the_run() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    let slot = fixture.v2_store.result_path(&summary.calls[0].id);
    std::fs::remove_file(&slot).unwrap();
    std::fs::create_dir(&slot).unwrap();

    let error = try_finalize_with(&fixture, &scripted(&fixture, 0), summary)
        .await
        .expect_err("an unreadable record pauses the run");

    let message = paused_message(&error).unwrap_or_else(|| panic!("{error:#}"));
    assert!(message.contains("cannot be read"), "{message}");
    assert_eq!(status(&fixture), RunStatus::Paused);
}
