use super::*;
use crate::command::workflow_host_fault_test_hooks::{Point, inject};

#[tokio::test]
async fn host_command_committed_observation_does_not_execute_or_publish_twice() {
    let fixture = fixture(0, false);
    let fault = inject(
        Point::AfterCommit,
        &fixture.executor.context_for_test().task_root,
        Box::new(|| Err(std::io::Error::from(std::io::ErrorKind::Interrupted))),
    );
    let outcome = run(&fixture).await;
    assert!(
        fault.fired(),
        "the fixture fault did not reach host execution"
    );
    outcome.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let recorded = records(&fixture);
    assert_eq!(recorded.len(), 1);
    let receipt = &recorded[0].result.data["publicationReceipt"];
    assert!(receipt.is_object());
    assert!(
        recorded[0].result.data["gateEnvelope"]["operational_error"].is_null(),
        "{:?}",
        recorded[0].result.data
    );
    assert_eq!(recorded[0].result.data["postcondition"]["satisfied"], true);
    assert!(
        receipt["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["prior_blake3"].is_null())
    );
}

struct DeferredTeardown {
    inner: Arc<RecoveringProcess>,
    token: std::sync::Mutex<
        Option<crate::command::workflow_host_command_teardown_latch::TeardownToken>,
    >,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for DeferredTeardown {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        if self.inner.calls.load(Ordering::SeqCst) == 0 {
            *self.token.lock().unwrap() = Some(control.track_teardown());
            self.inner.calls.fetch_add(1, Ordering::SeqCst);
            self.inner.started.notify_one();
            return Err(WorkflowError::port(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )));
        }
        self.inner.execute(request, control).await
    }
}

#[tokio::test]
async fn host_command_retry_waits_for_confirmed_teardown() {
    let mut fixture = fixture(0, false);
    let deferred = Arc::new(DeferredTeardown {
        inner: fixture.process.clone(),
        token: Default::default(),
    });
    Arc::get_mut(&mut fixture.executor)
        .unwrap()
        .replace_process_for_test(deferred.clone());
    let work = run(&fixture);
    tokio::pin!(work);
    tokio::select! {
        biased;
        result = &mut work => panic!("escaped teardown: {result:?}"),
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {},
    }
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    deferred.token.lock().unwrap().take().unwrap().settled(true);
    work.await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn host_command_unknown_gate_fault_keeps_its_exact_text_once() {
    let fixture = fixture(0, false);
    *fixture.process.evaluation.lock().unwrap() = Some(
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &anyhow::anyhow!("record has invalid shape; restore record.json"),
        ),
    );
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let recorded = records(&fixture);
    let error = &recorded[0].result.data["gateEnvelope"]["operational_error"];
    assert_eq!(error["kind"], "permanent_host_fault");
    assert_eq!(
        error["text"],
        "record has invalid shape; restore record.json"
    );
}

#[tokio::test]
async fn host_command_validation_fault_is_typed_permanent_at_producer() {
    let fixture = fixture(0, false);
    let stamp = archon_workflow::task_set_contract::FreezeGateStamp {
        mode: archon_workflow::task_set_contract::FreezeGateMode::Enforce,
        evaluated_at: "".into(),
        binary_commit: "".into(),
        finding_count: 0,
        findings_digest: "".into(),
    };
    let error = archon_workflow::task_set_contract::validate_gate_stamp(&stamp, "acceptance.lock")
        .unwrap_err();
    *fixture.process.evaluation.lock().unwrap() = Some(
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &error.into(),
        ),
    );
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let recorded = records(&fixture);
    let error = &recorded[0].result.data["gateEnvelope"]["operational_error"];
    assert_eq!(error["kind"], "permanent_host_fault");
    assert!(error["text"].as_str().unwrap().contains(
        "acceptance.lock gate provenance is incomplete; re-freeze with the current binary"
    ));
}

#[tokio::test]
async fn host_command_fresh_staging_interruption_recovers_without_pause() {
    let fixture = fixture(0, false);
    let fault = inject(
        Point::Prepare,
        &fixture.store.run_dir(&fixture.run_id),
        Box::new(|| Err(std::io::Error::from(std::io::ErrorKind::Interrupted))),
    );
    let outcome = run(&fixture).await;
    assert!(
        fault.fired(),
        "the fixture fault did not reach host execution"
    );
    outcome.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}

struct ExitBeforeEnvelope(Arc<RecoveringProcess>);
#[async_trait::async_trait]
impl HostCommandProcessAdapter for ExitBeforeEnvelope {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        if self.0.calls.load(Ordering::SeqCst) == 0 {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            let stdout = serde_json::to_vec(&crate::command::workflow_host_fault::envelope(
                &anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
            ))
            .unwrap();
            return Ok(SupervisedProcessOutput {
                exit_code: Some(1),
                timed_out: false,
                stdout_bytes: stdout.len() as u64,
                stdout_retained_bytes: stdout.len() as u64,
                stdout,
                stderr: b"judge transport connection reset".to_vec(),
                stderr_bytes: 32,
                stderr_retained_bytes: 32,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_path: None,
                stderr_path: None,
            });
        }
        self.0.execute(request, control).await
    }
}

#[tokio::test]
async fn host_command_pre_envelope_transport_recovers() {
    let mut fixture = fixture(0, false);
    let adapter = Arc::new(ExitBeforeEnvelope(fixture.process.clone()));
    Arc::get_mut(&mut fixture.executor)
        .unwrap()
        .replace_process_for_test(adapter);
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}

#[tokio::test]
async fn host_command_sealing_interruption_recovers_after_safe_removal() {
    let fixture = fixture(0, false);
    let fault = inject(
        Point::Seal,
        &fixture.store.run_dir(&fixture.run_id),
        Box::new(|| Err(std::io::Error::from(std::io::ErrorKind::Interrupted))),
    );
    let outcome = run(&fixture).await;
    assert!(
        fault.fired(),
        "the fixture fault did not reach host execution"
    );
    outcome.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}

#[tokio::test]
async fn host_command_commit_flush_interruption_keeps_receipt_without_second_publication() {
    let fixture = fixture(0, false);
    let context = fixture.executor.context_for_test();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(
        &context.project_root,
        &context.task_root,
    );
    let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin);
    let fault = inject(
        Point::CommitSync,
        &journal,
        Box::new(|| Err(std::io::Error::from(std::io::ErrorKind::Interrupted))),
    );
    let outcome = run(&fixture).await;
    assert!(
        fault.fired(),
        "the fixture fault did not reach host execution"
    );
    outcome.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let recorded = records(&fixture);
    assert!(recorded[0].result.data["publicationReceipt"].is_object());
    assert!(
        recorded[0].result.data["publicationReceipt"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["prior_blake3"].is_null())
    );
}

#[tokio::test]
async fn host_command_pause_after_commit_resumes_observation_without_execution() {
    let fixture = fixture(0, false);
    let published = Arc::new(tokio::sync::Notify::new());
    let signal = published.clone();
    let fault = inject(
        Point::AfterCommit,
        &fixture.executor.context_for_test().task_root,
        Box::new(move || {
            signal.notify_one();
            Err(std::io::Error::from(std::io::ErrorKind::Interrupted))
        }),
    );
    let work = run(&fixture);
    tokio::pin!(work);
    tokio::select! {
        biased;
        result = &mut work => panic!("post-commit fault escaped before pause: {result:?}"),
        _ = published.notified() => {},
    }
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    let first = work.await;
    assert!(
        fault.fired(),
        "the fixture fault did not reach host execution"
    );
    assert!(
        first.is_ok() || matches!(&first, Err(WorkflowError::ControlPaused(_))),
        "{first:?}"
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        archon_workflow::run::RunStatus::Paused
    );
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    assert!(records(&fixture).last().unwrap().result.data["publicationReceipt"].is_object());
}

struct InternalTimeout(Arc<RecoveringProcess>);
#[async_trait::async_trait]
impl HostCommandProcessAdapter for InternalTimeout {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        if self.0.calls.load(Ordering::SeqCst) < 2 {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            return Ok(SupervisedProcessOutput {
                exit_code: None,
                timed_out: true,
                stdout: Vec::new(),
                stderr: Vec::new(),
                stdout_bytes: 0,
                stdout_retained_bytes: 0,
                stderr_bytes: 0,
                stderr_retained_bytes: 0,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_path: None,
                stderr_path: None,
            });
        }
        self.0.execute(request, control).await
    }
}

#[tokio::test]
async fn host_command_internal_timeout_recovers_before_recording() {
    let mut fixture = fixture(0, false);
    let adapter = Arc::new(InternalTimeout(fixture.process.clone()));
    Arc::get_mut(&mut fixture.executor)
        .unwrap()
        .replace_process_for_test(adapter);
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}

#[path = "workflow_host_command_heal_fault_tests.rs"]
mod faults;
#[path = "workflow_host_command_heal_site_tests.rs"]
mod sites;

#[tokio::test]
async fn host_command_task_set_error_after_commit_keeps_receipt_on_resume() {
    let fixture = fixture(0, false);
    let fault = inject(
        Point::AfterCommit,
        &fixture.executor.context_for_test().task_root,
        Box::new(|| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))),
    );
    run(&fixture).await.unwrap();
    assert!(fault.fired());
    let before = records(&fixture);
    assert!(before[0].result.data["publicationReceipt"].is_object());
    assert_eq!(
        before[0].result.data["gateEnvelope"]["operational_error"]["kind"],
        "permanent_host_fault"
    );
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let after = records(&fixture);
    assert_eq!(
        before[0].result.data["publicationReceipt"],
        after[0].result.data["publicationReceipt"]
    );
    assert!(after[0].result.data["gateEnvelope"]["operational_error"].is_null());
}

#[tokio::test]
async fn host_command_binding_missing_skeleton_is_host_fault_not_candidate_refusal() {
    use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;
    let fixture = fixture(0, false);
    let context = fixture.executor.context_for_test();
    std::fs::remove_file(
        context
            .task_root
            .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
    )
    .unwrap();
    let request =
        archon_workflow::HostCommandRequest::new("land-task-body", Some("candidate".into()))
            .unwrap();
    let result = fixture
        .executor
        .execute(
            request,
            Some(
                fixture
                    .store
                    .load_state(&fixture.run_id)
                    .unwrap()
                    .generation,
            ),
        )
        .await
        .unwrap();
    let envelope = result.gate_envelope.unwrap();
    assert_eq!(
        envelope.operational_error.unwrap().kind,
        "permanent_host_fault"
    );
    assert!(
        envelope
            .policy_findings
            .iter()
            .all(|finding| finding.remediation_scope
                == archon_workflow::RemediationScope::Operational)
    );
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 0);
}

#[path = "workflow_decompose_launch_input_tests.rs"]
mod workflow_decompose_launch_input;
