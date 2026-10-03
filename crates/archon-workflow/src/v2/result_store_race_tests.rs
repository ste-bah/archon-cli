//! Deterministic restart between branch computation/selection and persistence.
use super::*;

thread_local! {
    static BEFORE_SAVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

pub(crate) fn on_branch_save(action: impl FnOnce() + 'static) {
    BEFORE_SAVE.with(|hook| *hook.borrow_mut() = Some(Box::new(action)));
}

pub(crate) fn before_branch_save() {
    let action = BEFORE_SAVE.with(|hook| hook.borrow_mut().take());
    if let Some(action) = action {
        action();
    }
}

#[test]
fn a_stale_branch_save_waits_for_the_restart_lock_and_refuses_its_epoch() {
    let temp = tempfile::tempdir().unwrap();
    let runs = crate::WorkflowStore::new(temp.path());
    let v2 = WorkflowV2ResultStore::new(runs.run_dir("run").join("v2"));
    let outcome = WorkflowV2BranchOutcome {
        item_id: "one".into(),
        role: "worker".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(WorkflowV2Result::accepted("done")),
        error: None,
        failure_kind: None,
        item_input_hash: Some("input".into()),
        completion_evidence: vec![],
    };
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    // Start the old write while restart holds the lock, before advancing the epoch.
    let worker = runs.with_run_lock("run", |_| {
        let old = v2.clone();
        let worker = std::thread::spawn(move || {
            on_branch_save(move || entered_tx.send(()).unwrap());
            done_tx
                .send(old.save_branch_outcome("fanout", &outcome))
                .unwrap();
        });
        entered_rx.recv().unwrap();
        assert!(
            done_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "branch persistence must wait for the restart's run lock"
        );
        v2.bump_restart_epoch()?;
        Ok(worker)
    });
    let worker = worker.unwrap();
    worker.join().unwrap();
    assert!(matches!(
        done_rx.recv().unwrap(),
        Err(WorkflowError::ControlCancelled(_))
    ));
    assert!(v2.load_branch_outcome("fanout", "one").unwrap().is_none());
}
