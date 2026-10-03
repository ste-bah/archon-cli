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
