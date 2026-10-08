//! Issue 261: a pause request belongs to the executor that makes it. Once a
//! resume hands the run to a newer executor generation, an older executor
//! still running its script cannot pause the run: it is refused as stale, and
//! the run, its generation and its event log stay as the new owner left them.
use super::*;

#[tokio::test]
async fn a_stale_executor_cannot_pause_a_run_a_newer_generation_owns() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let bound = store.load_state(&run_id).unwrap().generation;
    let (runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    // What `run` binds when the executor starts under `bound`.
    runner.v2_store.bind_session_executor(bound);
    let host = WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    };
    // Model a newer executor generation without changing the restart epoch,
    // so this exercises the ownership fence rather than the restart fence.
    let mut owner = store.load_state(&run_id).unwrap();
    owner.generation = owner.generation.saturating_add(1);
    owner.executor_generation = Some(owner.generation);
    store.save_state(&owner).unwrap();
    let owner = store.load_state(&run_id).unwrap();
    assert_eq!(owner.status, archon_workflow::RunStatus::Running);
    assert!(
        owner.executor_generation.is_some_and(|g| g > bound),
        "the resume took the run over: {:?}",
        owner.executor_generation
    );
    // Give this same id valid replay credit from an earlier generation. The
    // stale host would incorrectly return `Passed` if the ownership fence were
    // absent from that branch.
    let pause_dir = store.run_dir(&run_id).join("v2/script-pauses");
    std::fs::create_dir_all(&pause_dir).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(b"pause-stale-1");
    let record_path = pause_dir.join(format!("pause-stale-1-{}.json", &digest[..16]));
    let prior_pause = serde_json::json!({
        "pause_id": "pause-stale-1", "joined": false, "event_seq": 1,
        "generation": bound, "covered": [], "host_taken": false,
    });
    std::fs::write(&record_path, serde_json::to_vec(&prior_pause).unwrap()).unwrap();

    let outcome = host
        .request_script_pause_for_test(
            &serde_json::json!({
                "id": "pause-stale-1",
                "options": { "evidence": { "subject": "stale", "reason": "no_progress" } },
            })
            .to_string(),
        )
        .await;

    let after = store.load_state(&run_id).unwrap();
    assert_eq!(
        (after.status.clone(), after.generation),
        (archon_workflow::RunStatus::Running, owner.generation),
        "a stale executor paused the run generation {} owns",
        owner.generation
    );
    assert!(
        matches!(&outcome, Err(WorkflowError::ControlCancelled(message)) if message.contains("no longer owns")),
        "the stale caller is refused, never answered: {outcome:?}"
    );
    assert!(pause_events(&store, &run_id).is_empty());
    assert!(
        record_path.exists(),
        "the prior generation's credit remains available for this stale-path check"
    );
}

#[tokio::test]
async fn a_saved_pause_reports_when_its_evidence_event_cannot_be_written() {
    let (_temp, store, run_id) = super::new_run();
    super::set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let (runner, _rx) = super::runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    let host = WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    };
    // A directory at the event-log path makes the real append fail after the
    // run state is saved as paused.
    std::fs::remove_file(store.events_path(&run_id)).unwrap();
    std::fs::create_dir(store.events_path(&run_id)).unwrap();
    let outcome = host
        .execute(
            archon_workflow::v2::script::SCRIPT_PAUSE_METHOD.into(),
            serde_json::json!({"id":"pause-no-event","options":{}}).to_string(),
        )
        .await;
    assert!(
        matches!(&outcome, Err(WorkflowError::SpecInvalid(message)) if message.contains("evidence event was not recorded")),
        "the missing evidence is explicit: {outcome:?}"
    );
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Paused,
        "the state transition may have saved before evidence failed"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn a_pause_with_event_but_no_replay_record_reports_the_gap() {
    let (_temp, store, run_id) = super::new_run();
    super::set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let (runner, _rx) = super::runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    let host = WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    };
    let pause_dir = store.run_dir(&run_id).join("v2/script-pauses");
    std::fs::create_dir_all(pause_dir.parent().unwrap()).unwrap();
    std::fs::create_dir(&pause_dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&pause_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let outcome = host
        .execute(
            archon_workflow::v2::script::SCRIPT_PAUSE_METHOD.into(),
            serde_json::json!({"id":"pause-no-record","options":{}}).to_string(),
        )
        .await;
    std::fs::set_permissions(&pause_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(&outcome, Err(WorkflowError::SpecInvalid(message)) if message.contains("replay record was not written")),
        "the pause reports its missing replay record: {outcome:?}"
    );
    assert_eq!(super::pause_events(&store, &run_id).len(), 1);
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
}
