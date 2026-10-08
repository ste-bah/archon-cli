//! Coverage replaced while a pause request waits for the run lock cannot earn credit.
use super::*;

#[tokio::test]
async fn round3_coverage_is_validated_when_the_pause_request_owns_the_run_lock() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    run_script(&store, &run_id, PAUSING_SCRIPT)
        .await
        .expect_err("seed a pause covering before-pause");
    resume(&store, &run_id);
    let (runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    let host = WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    };
    let (entered, started) = std::sync::mpsc::channel();
    let caller = store
        .with_run_lock(&run_id, |_| {
            let caller = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                runtime.block_on(Box::pin(async {
                    entered.send(()).unwrap();
                    Box::pin(host.execute(
                        archon_workflow::v2::script::SCRIPT_PAUSE_METHOD.into(),
                        serde_json::json!({"id":"pause-subject-1","options":{}}).to_string(),
                    ))
                    .await
                }))
            });
            started.recv().unwrap();
            // Let the caller reach the held lock. Its old unlocked validation
            // could finish here, but no locked validation or grant can finish.
            std::thread::sleep(std::time::Duration::from_millis(100));
            WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
                .invalidate_call_and_dependents(&[], "before-pause")?;
            Ok(caller)
        })
        .unwrap();
    let outcome = caller.join().unwrap();
    assert!(
        matches!(&outcome, Err(WorkflowError::ControlPaused(_))),
        "replaced coverage must take a new pause instead of spending stale credit: {outcome:?}"
    );
    assert_eq!(pause_events(&store, &run_id).len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn waiting_for_the_run_lock_does_not_block_the_async_runtime() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let (runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    let host = WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    };
    let entered = std::sync::mpsc::channel();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let holder_done = done.clone();
    let holder_store = store.clone();
    let holder_run_id = run_id.clone();
    let holder = std::thread::spawn(move || {
        holder_store
            .with_run_lock(&holder_run_id, |_| {
                entered.0.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(160));
                Ok(())
            })
            .unwrap();
        holder_done.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    entered.1.recv().unwrap();
    let pause = tokio::spawn(async move {
        host.execute(
            archon_workflow::v2::script::SCRIPT_PAUSE_METHOD.into(),
            serde_json::json!({"id":"pause-lock-wait","options":{}}).to_string(),
        )
        .await
    });
    let timer_start = tokio::time::Instant::now();
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    assert!(
        timer_start.elapsed() < std::time::Duration::from_millis(100),
        "the host call blocked the current-thread runtime while waiting for the lock"
    );
    assert!(!done.load(std::sync::atomic::Ordering::SeqCst));
    let _ = pause.await.unwrap();
    holder.join().unwrap();
}
