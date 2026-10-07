//! Issue 337 round 5 (review findings 2 and 5): the pause a call's
//! unpersisted terminal stop takes is this session's own. A session that a
//! resume replaced pauses nothing (Issue 291); the owner's pause is fenced by
//! the generation it owns and records the coverage of the verdicts the run
//! holds.
use super::workflow_live_v2_script_pause_tests::{new_run, resume, runner, set_status};
use super::*;
use archon_workflow::{LifecycleAction, LifecycleController, RunStatus};

fn host_of(runner: WorkflowV2ScriptRunner) -> WorkflowScriptHost {
    WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    }
}

/// A final report that stopped the script for review, recorded in the run.
fn stop_record(store: &WorkflowStore, run_id: &str) -> WorkflowV2CallRecord {
    let record = WorkflowV2CallRecord::new(
        run_id.to_string(),
        WorkflowV2HostCall {
            id: "stopped".into(),
            method: WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "hash".into(),
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "stop for review".into(),
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    );
    WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .restore_call_record(&record)
        .unwrap();
    record
}

/// The stop record's path is taken by a directory: its write fails.
fn block_stop_record(store: &WorkflowStore, run_id: &str) {
    std::fs::create_dir_all(store.run_dir(run_id).join("v2/terminal-stop.json/held")).unwrap();
}

/// A host bound to the run's executor generation now (raw outcomes on).
fn owner_host(store: &WorkflowStore, run_id: &str) -> (WorkflowScriptHost, u64, impl Sized) {
    set_status(store, run_id, RunStatus::Running);
    let bound = store.load_state(run_id).unwrap().generation;
    let (runner, rx) = runner(store, run_id, Arc::new(PanicLlm), None, None);
    runner.v2_store.bind_session_executor(bound);
    (host_of(runner), bound, rx)
}

fn files(store: &WorkflowStore, run_id: &str) -> (Vec<u8>, Vec<u8>) {
    (
        std::fs::read(store.state_path(run_id)).unwrap(),
        std::fs::read(store.events_path(run_id)).unwrap_or_default(),
    )
}

fn coverage_records(store: &WorkflowStore, run_id: &str) -> Vec<serde_json::Value> {
    std::fs::read_dir(store.run_dir(run_id).join("v2/script-pauses"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("host-terminal-stop-unpersisted-g")
        })
        .map(|entry| serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap())
        .collect()
}

/// The reviewer's case: a resume gave the run to a newer executor; the old
/// session's stop cannot be persisted (it no longer owns the run). It must
/// not pause the run the new executor owns, nor write anything into it.
#[tokio::test]
async fn a_replaced_session_never_pauses_the_run_for_its_unpersisted_stop() {
    for blocked in [false, true] {
        let (_temp, store, run_id) = new_run();
        let (host, bound, _rx) = owner_host(&store, &run_id);
        let record = stop_record(&store, &run_id);
        if blocked {
            block_stop_record(&store, &run_id);
        }
        LifecycleController::new(store.clone())
            .apply(&run_id, LifecycleAction::Pause)
            .unwrap();
        resume(&store, &run_id);
        let owner = store.load_state(&run_id).unwrap();
        assert!(owner.executor_generation.is_some_and(|g| g > bound));
        let before = files(&store, &run_id);

        host.persist_call_terminal_stop(&record);

        assert_eq!(
            files(&store, &run_id),
            before,
            "blocked={blocked}: the run state and events are untouched"
        );
        assert_ne!(store.load_state(&run_id).unwrap().status, RunStatus::Paused);
        assert!(coverage_records(&store, &run_id).is_empty());
        assert!(
            !store
                .run_dir(&run_id)
                .join("v2/terminal-stop.json")
                .is_file(),
            "blocked={blocked}: no stop is recorded for the new executor's generation"
        );
    }
}

/// The owner's stop cannot be persisted: it pauses at the generation it owns,
/// with the coverage of the verdict that decided the stop.
#[tokio::test]
async fn the_owner_pauses_its_unpersisted_stop_with_the_coverage_of_its_verdict() {
    let (_temp, store, run_id) = new_run();
    let (host, bound, _rx) = owner_host(&store, &run_id);
    let record = stop_record(&store, &run_id);
    block_stop_record(&store, &run_id);

    host.persist_call_terminal_stop(&record);

    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    let pause = events
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["detail"]["event"] == "terminal_stop_unpersisted")
        .expect("the pause carries the refusal");
    assert_eq!(pause["detail"]["paused_by_generation"], bound);
    let coverage = coverage_records(&store, &run_id);
    assert_eq!(coverage.len(), 1, "{coverage:?}");
    assert_eq!(coverage[0]["host_taken"], true);
    assert_eq!(coverage[0]["covered"][0]["call_id"], "stopped");
}

/// The operator paused the run before the stop: nothing is persisted (no
/// running run) and nothing pauses it again; the operator's pause stands.
#[tokio::test]
async fn an_unpersisted_stop_never_pauses_a_generation_its_session_did_not_own() {
    let (_temp, store, run_id) = new_run();
    let (host, bound, _rx) = owner_host(&store, &run_id);
    let record = stop_record(&store, &run_id);
    block_stop_record(&store, &run_id);
    // The operator paused the run: the stop is not persisted (no running
    // run), and nothing pauses it again.
    LifecycleController::new(store.clone())
        .apply(&run_id, LifecycleAction::Pause)
        .unwrap();
    let before = files(&store, &run_id);

    host.persist_call_terminal_stop(&record);

    assert_eq!(files(&store, &run_id), before);
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert!(run.generation > bound);
    assert!(coverage_records(&store, &run_id).is_empty());
}
