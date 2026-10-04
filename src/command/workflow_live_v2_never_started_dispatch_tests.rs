//! Issue 318: the Issue 263 never-started streak, driven through the host's
//! own `execute()`. A merge once routed the never-started fault away from the
//! streak and every test still passed; these hold the whole path.

use super::workflow_live_v2_reuse_content_key_tests::{reuse_test_runner, reuse_test_store};
use super::*;

const UPSTREAM: &str = "upstream";

/// A run whose `upstream` record is wrongly shaped, so every call that
/// reads it as its source fails before it starts (a `StateCorrupt` fault),
/// and a host session on it.
fn session() -> (
    tempfile::TempDir,
    WorkflowStore,
    archon_workflow::WorkflowRun,
    WorkflowV2ResultStore,
    WorkflowScriptHost,
) {
    let temp = tempfile::tempdir().expect("tempdir");
    let (store, mut run) = reuse_test_store(&temp);
    run.status = archon_workflow::RunStatus::Running;
    store.save_state(&run).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let slot = v2.result_path(UPSTREAM);
    std::fs::create_dir_all(slot.parent().unwrap()).unwrap();
    std::fs::write(
        &slot,
        r#"{"call":{"id":"upstream"},"attempt":"one","input_hash":1,"status":"accepted","result":{}}"#,
    )
    .unwrap();
    let host = WorkflowScriptHost {
        host_occurrences: Default::default(),
        scaffold_hash: "fixture".into(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner: reuse_test_runner(&store, &run, &v2, serde_json::Value::Null, None),
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
        tool_host: std::sync::OnceLock::new(),
        tool_budget: Default::default(),
    };
    (temp, store, run, v2, host)
}

async fn probe(host: &WorkflowScriptHost, id: &str) -> archon_workflow::WorkflowResult<String> {
    let payload = serde_json::json!({ "id": id, "options": { "source": UPSTREAM } });
    Box::pin(host.execute("checkpoint".into(), payload.to_string())).await
}

/// N consecutive dispatches that never start: the first is a failed stage
/// the script sees, the one at the limit pauses the run with evidence, and
/// that call's interrupted record is saved for the resume.
#[tokio::test]
async fn consecutive_never_started_dispatches_pause_the_run_and_save_the_interrupted_call() {
    let (_temp, store, run, v2, host) = session();
    let limit = archon_workflow::v2::host_fault::NeverStartedStreak::DEFAULT_LIMIT;
    for n in 1..limit {
        let view = probe(&host, &format!("probe-{n}")).await;
        assert!(
            view.is_ok(),
            "below the limit the script sees a failed stage: {view:?}"
        );
    }

    let stopped = probe(&host, &format!("probe-{limit}")).await;

    assert!(
        matches!(&stopped, Err(WorkflowError::ControlPaused(message)) if message.contains("never started")),
        "{stopped:?}"
    );
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    let interrupted = v2
        .load_call_record(&format!("probe-{limit}"))
        .unwrap()
        .expect("the interrupted call is recorded");
    assert!(
        interrupted.result.data["interrupted"].is_string(),
        "{interrupted:?}"
    );
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(events.contains("never_started_pause"), "{events}");
}

/// After a terminal host stop (#285) the run's outcome is that stop: a
/// never-started streak in a sibling call never writes a pause over it.
#[tokio::test]
async fn a_never_started_streak_after_a_terminal_stop_never_pauses_the_run() {
    let (_temp, store, run, _v2, host) = session();
    host.accumulator.lock().await.terminal_host_stop = true;
    let generation = Some(store.load_state(&run.id).unwrap().generation);
    let limit = archon_workflow::v2::host_fault::NeverStartedStreak::DEFAULT_LIMIT;
    for n in 1..=limit {
        let fault = WorkflowError::StateCorrupt("results/upstream.json: wrong shape".into());

        let error = host
            .pause_on_never_started_streak(&format!("probe-{n}"), generation, fault)
            .await;

        assert!(matches!(error, WorkflowError::StateCorrupt(_)), "{error:?}");
    }
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        archon_workflow::RunStatus::Running
    );
}
