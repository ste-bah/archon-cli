//! Issue 320, through the real host: in a run with a task set, acceptance
//! rounds that only hit operational errors never end the loop. The host's
//! no-progress ledger pauses the run, and a resumed run goes on with the loop.
use super::*;

const SCRIPT: &str = r#"
export const meta = { name: 'operational-acceptance', phases: [{ title: 'One' }] }
export default async function workflow({ phase, log }) {
  await phase("Work");
  await log("done");
  return { accepted: [], blocked: [], notes: "work done" };
}
"#;

fn runner_for(
    store: &WorkflowStore,
    run_id: &str,
    ui_sink: archon_workflow::SharedWorkflowUiSink,
) -> WorkflowV2ScriptRunner {
    let spec = test_spec();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run_id.into(),
        None,
        None,
    );
    WorkflowV2ScriptRunner::new(
        "operational acceptance".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2")),
        store.clone(),
        run_id.to_string(),
        true,
        Some(task_universe()),
        None,
    )
}

fn acceptance_attempts(store: &WorkflowStore, run_id: &str) -> Vec<String> {
    let root = store.run_dir(run_id).join("v2/acceptance");
    let mut found = Vec::new();
    for round in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        let name = round.file_name().to_string_lossy().to_string();
        if !name.starts_with("round-") {
            continue;
        }
        for attempt in std::fs::read_dir(round.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let file = attempt.file_name().to_string_lossy().to_string();
            if file.starts_with("attempt-") && file.ends_with(".json") {
                found.push(format!("{name}/{file}"));
            }
        }
    }
    found.sort();
    found
}

/// The fixture has a task set but no project, so every acceptance round
/// records an operational error (the stage context will not resolve) and no
/// check. The run pauses on the host's stall at
/// round 3 (never ends after round 1, never fails), and the resume runs the
/// loop again instead of ending it: a new acceptance attempt is recorded, and
/// with nothing changed the run is paused again, not ended.
#[tokio::test]
async fn operational_only_rounds_pause_on_the_host_stall_and_the_resume_goes_on() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(test_spec()).expect("run");
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let first = runner_for(&store, &run.id, ui_sink.clone())
        .run(SCRIPT)
        .await;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    assert!(
        matches!(&first, Err(WorkflowError::ControlPaused(message))
            if message.contains("acceptance round 3") && message.contains("made no progress")),
        "{first:?}"
    );
    for round in 1..=2 {
        let id = format!("acceptance-contract-run-{round}");
        assert!(v2_store.load_call_record(&id).unwrap().is_some(), "{id}");
    }
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    let pause = events
        .lines()
        .find(|line| line.contains("acceptance_stall_pause"))
        .expect("the stall pause carries its evidence");
    assert!(pause.contains("operational_errors"), "{pause}");
    let before = acceptance_attempts(&store, &run.id);
    assert_eq!(
        before,
        [
            "round-01/attempt-01.json",
            "round-02/attempt-01.json",
            "round-03/attempt-01.json"
        ]
    );

    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let resumed = runner_for(&store, &run.id, ui_sink).run(SCRIPT).await;
    let after = acceptance_attempts(&store, &run.id);
    assert!(
        after.len() > before.len(),
        "the resumed run went on with the acceptance loop: {after:?}"
    );
    assert!(
        matches!(&resumed, Err(WorkflowError::ControlPaused(message)) if message.contains("made no progress")),
        "with nothing changed the resumed loop pauses again, never ends: {resumed:?}"
    );
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
}
