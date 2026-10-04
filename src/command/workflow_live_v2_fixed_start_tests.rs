use super::*;

#[test]
fn round3_fixed_start_is_serialized_with_lifecycle_stop() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store
        .create_run(super::super::workflow_run_finalizer_tests::spec())
        .unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let mut worker = None;
    let blocked = store
        .with_run_lock(&run.id, |locked| {
            let worker_store = store.clone();
            let mut startup = run.clone();
            worker = Some(std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                done_tx
                    .send(persist_fixed_start(&worker_store, &mut startup))
                    .unwrap();
            }));
            started_rx.recv().unwrap();
            let blocked = done_rx
                .recv_timeout(std::time::Duration::from_millis(200))
                .is_err();
            // Simulate the lifecycle transition while it owns the control lock.
            let mut stopped = locked.load_state(&run.id)?;
            stopped.status = RunStatus::Paused;
            stopped.generation += 1;
            locked.save_state(&stopped)?;
            Ok(blocked)
        })
        .unwrap();
    worker.unwrap().join().unwrap();
    assert!(
        blocked,
        "startup wrote state while a lifecycle transition held the control lock"
    );
    assert!(
        done_rx.recv().unwrap().is_err(),
        "the stale startup must be rejected"
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
}
