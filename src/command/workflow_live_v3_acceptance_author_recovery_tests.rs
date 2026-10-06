use super::super::repair_tests::{Run, run_fixture_with};
use super::*;
use archon_workflow::control_pause::PauseOwner;
use archon_workflow::stage_write::{self, StageWriter};
use archon_workflow::{LifecycleAction, LifecycleController, RunStatus, WorkflowError};

thread_local! {
    static ENTRY: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn entry_hook() {
    if let Some(hook) = ENTRY.with(|entry| entry.borrow_mut().take()) {
        hook();
    }
}
fn setup(state: &str) -> (Run, StageContext, StageWriter, Vec<u8>) {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let mut saved = run.store.load_state(&run.run_id).unwrap();
    saved.status = RunStatus::Running;
    run.store.save_state(&saved).unwrap();
    let writer = StageWriter {
        store: run.store.clone(),
        run_id: run.run_id.clone(),
        owner: PauseOwner::Generation(saved.generation),
    };
    let context = super::super::exec::resolve_context(
        &run.store,
        &run.run_id,
        run.runtime.target_repository_root.as_deref(),
        Some(&run.universe),
    )
    .unwrap();
    let pin = run.set.pin_path();
    let before = std::fs::read(&pin).unwrap();
    let mut new: serde_json::Value = serde_json::from_slice(&before).unwrap();
    new["freeze_event_id"] = "interrupted-publication".into();
    let _fix = crate::command::workflow_task_set::crash_publish(
        &pin,
        &[(pin.clone(), serde_json::to_vec(&new).unwrap())],
        state,
        false,
    );
    assert!(archon_workflow::task_set_publish_lock::interrupted_publish_left(&pin));
    (run, context, writer, before)
}
async fn heal(run: &Run, context: &StageContext) -> WorkflowResult<Option<Authored>> {
    let root = run.store.run_dir(&run.run_id);
    heal_unfrozen(&Site {
        llm: None,
        context,
        run_dir: &root,
        base: None,
        store: &run.store,
        run_id: &run.run_id,
        call_id: "recovery-entry",
    })
    .await
}
async fn refused(state: &str, resume: bool) {
    let (run, context, writer, before) = setup(state);
    let pin = run.set.pin_path();
    let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin);
    let journal_before = std::fs::read(&journal).unwrap();
    let log = archon_workflow::task_set_publish_lock::recovery_log_path(&pin);
    let log_before = std::fs::read(&log).ok();
    let control = LifecycleController::new(run.store.clone());
    control.apply(&run.run_id, LifecycleAction::Pause).unwrap();
    if resume {
        control.apply(&run.run_id, LifecycleAction::Resume).unwrap();
    }
    let result = stage_write::scope(writer, heal(&run, &context)).await;
    // Check this entry's transaction mutation first, not a shared result assertion.
    assert!(
        std::fs::read(&journal).ok().as_deref() == Some(journal_before.as_slice()),
        "initial recovery consumed the interrupted journal after ownership stopped"
    );
    assert_eq!(std::fs::read(&pin).unwrap(), before);
    assert_eq!(std::fs::read(log).ok(), log_before);
    assert!(matches!(
        result,
        Err(WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_))
    ));
}
#[tokio::test]
async fn r2_initial_recovery_paused_prepared_journal() {
    refused("prepared", false).await;
}
#[tokio::test]
async fn r2_initial_recovery_obsolete_committed_journal() {
    refused("committed", true).await;
}
#[tokio::test]
async fn r2_initial_recovery_obsolete_prepared_journal() {
    refused("prepared", true).await;
}

#[test]
fn r2_initial_recovery_takeover_waits_for_acquisition_and_settlement() {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let (run, context, writer, _) = setup("committed");
    let pin = run.set.pin_path();
    // A publisher holds the lock while the initial recovery starts.
    let publication = archon_workflow::task_set_publish_lock::PublishLockFile::acquire(
        &archon_workflow::task_set_publish_lock::lock_path(&pin),
    )
    .unwrap();
    let (entered_tx, entered_rx) = channel();
    let (continue_tx, continue_rx) = channel();
    let store = run.store.clone();
    let id = run.run_id.clone();
    let worker = std::thread::spawn(move || {
        ENTRY.with(|entry| {
            *entry.borrow_mut() = Some(Box::new(move || {
                entered_tx.send(()).unwrap();
                continue_rx.recv_timeout(Duration::from_secs(10)).unwrap();
            }))
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let recovered = runtime
            .block_on(stage_write::scope(writer, heal(&run, &context)))
            .is_ok();
        (recovered, run)
    });
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    continue_tx.send(()).unwrap();
    let (started_tx, started_rx) = channel();
    let (done_tx, done_rx) = channel();
    let takeover = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let control = LifecycleController::new(store);
        control.apply(&id, LifecycleAction::Pause).unwrap();
        control.apply(&id, LifecycleAction::Resume).unwrap();
        done_tx.send(()).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let interleaved = done_rx.recv_timeout(Duration::from_millis(200)).is_ok();
    drop(publication);
    let (recovered, _run) = worker.join().unwrap();
    takeover.join().unwrap();
    assert!(
        !interleaved,
        "takeover completed while initial recovery waited for publication"
    );
    assert!(recovered);
    assert!(!archon_workflow::task_set_publish_lock::interrupted_publish_left(&pin));
    let new: serde_json::Value = serde_json::from_slice(&std::fs::read(pin).unwrap()).unwrap();
    assert_eq!(new["freeze_event_id"], "interrupted-publication");
}
