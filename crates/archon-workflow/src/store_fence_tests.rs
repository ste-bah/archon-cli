//! Issue 291 (round 4): the fence never holds the run lock across a poll. A
//! pause, cancel or resume lands within seconds while a long synchronous
//! verifier runs inside fenced work, and the fence then acts on it.
use super::*;
use crate::{LifecycleAction, LifecycleController};
use std::time::{Duration, Instant};

/// How long the stand-in verifier blocks its poll (the real one may block up
/// to 900 s); the control command must land far sooner.
const VERIFIER: Duration = Duration::from_secs(4);
const PROMPT: Duration = Duration::from_millis(1500);

fn running_run() -> (tempfile::TempDir, WorkflowStore, String, u64) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = crate::WorkflowSpec {
        schema: crate::spec::WORKFLOW_SCHEMA.into(),
        name: "fence".into(),
        task: "test".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    };
    let mut run = store.create_run(spec).unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();
    let generation = run.generation;
    (temp, store, run.id, generation)
}

/// Run `kind`-fenced work whose single poll blocks like the artifact verifier
/// (`try_wait` + `thread::sleep`), then apply `actions` from this thread and
/// time each. Returns the work's result and the elapsed control times.
fn control_during_verifier(
    kind: FenceKind,
    actions: &[LifecycleAction],
) -> (
    WorkflowResult<&'static str>,
    Vec<Duration>,
    WorkflowStore,
    String,
    tempfile::TempDir,
) {
    let (temp, store, id, generation) = running_run();
    let bound = store.for_executor(&id, generation);
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let work = {
        let (bound, id) = (bound.clone(), id.clone());
        runtime.spawn(async move {
            let verifier = async {
                entered_tx.send(()).unwrap();
                std::thread::sleep(VERIFIER);
                Ok("verified")
            };
            match kind {
                FenceKind::Admission => bound.execute_owned(&id, verifier).await,
                FenceKind::Ownership => bound.execute_writer(&id, verifier).await,
            }
        })
    };
    entered.recv_timeout(Duration::from_secs(30)).unwrap();
    let lifecycle = LifecycleController::new(store.clone());
    let timings = actions
        .iter()
        .map(|action| {
            let started = Instant::now();
            lifecycle.apply(&id, action.clone()).unwrap();
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let result = runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(60), work).await })
        .expect("fenced work must end")
        .unwrap();
    (result, timings, bound, id, temp)
}

fn assert_prompt(timings: &[Duration]) {
    for elapsed in timings {
        assert!(
            *elapsed < PROMPT,
            "run control waited {elapsed:?} behind a synchronous verifier poll"
        );
    }
}

#[test]
fn pause_lands_while_a_wave_verifier_runs_and_the_owner_unwinds() {
    let (result, timings, bound, id, _temp) =
        control_during_verifier(FenceKind::Ownership, &[LifecycleAction::Pause]);
    assert_prompt(&timings);
    // The paused owner keeps its verified work and may still record it.
    assert_eq!(result.unwrap(), "verified");
    assert!(
        bound.require_admission(&id).is_err(),
        "nothing new is admitted"
    );
    let run = bound.load_state(&id).unwrap();
    bound
        .save_state(&run)
        .expect("the paused owner preserves its evidence");
}

#[test]
fn cancel_lands_while_a_provider_poll_blocks_and_admission_then_stops() {
    let (result, timings, bound, id, _temp) =
        control_during_verifier(FenceKind::Admission, &[LifecycleAction::Cancel]);
    assert_prompt(&timings);
    // The poll that was already running finished: its result is evidence.
    assert_eq!(result.unwrap(), "verified");
    let next = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(bound.execute_owned(&id, async { Ok(()) }));
    assert!(matches!(next, Err(WorkflowError::ControlCancelled(_))));
}

#[test]
fn pause_and_resume_land_during_the_verifier_and_the_stale_owner_writes_nothing() {
    let (result, timings, bound, id, _temp) = control_during_verifier(
        FenceKind::Ownership,
        &[LifecycleAction::Pause, LifecycleAction::Resume],
    );
    assert_prompt(&timings);
    // The verifier's poll ended Ready; the stale owner's writes refuse.
    assert!(result.is_ok());
    let run = bound.load_state(&id).unwrap();
    assert!(matches!(
        bound.save_state(&run),
        Err(WorkflowError::ControlCancelled(_))
    ));
    let next = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(bound.execute_writer(&id, async { Ok(()) }));
    assert!(matches!(next, Err(WorkflowError::ControlCancelled(_))));
}

#[test]
fn a_pause_landing_before_the_next_poll_stops_admission_typed() {
    let (_temp, store, id, generation) = running_run();
    let bound = store.for_executor(&id, generation);
    LifecycleController::new(store.clone())
        .apply(&id, LifecycleAction::Pause)
        .unwrap();
    let polled = std::sync::atomic::AtomicBool::new(false);
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(bound.execute_owned(&id, async {
            polled.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }));
    assert!(matches!(result, Err(WorkflowError::ControlPaused(_))));
    assert!(!polled.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn a_removed_run_stops_its_executor_without_recreating_the_directory() {
    let (_temp, store, id, generation) = running_run();
    let bound = store.for_executor(&id, generation);
    std::fs::remove_dir_all(store.run_dir(&id)).unwrap();
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(bound.execute_owned(&id, async { Ok(()) }));
    assert!(matches!(result, Err(WorkflowError::ControlCancelled(_))));
    assert!(bound.require_admission(&id).is_err());
    assert!(!store.run_dir(&id).exists(), "the fence created nothing");
}
