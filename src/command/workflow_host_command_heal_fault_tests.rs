use super::*;

async fn assert_permanent_dispatch(error: anyhow::Error, artifact: &str) {
    let fixture = fixture(0, false);
    let original = format!("{error:#}");
    *fixture.process.evaluation.lock().unwrap() = Some(
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &error,
        ),
    );
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let recorded = records(&fixture);
    let fault = &recorded[0].result.data["gateEnvelope"]["operational_error"];
    assert_eq!(fault["kind"], "permanent_host_fault");
    assert!(fault["text"].as_str().unwrap().contains(artifact));
    assert!(fault["text"].as_str().unwrap().contains(&original));
}

#[tokio::test]
async fn host_command_malformed_repository_record_is_permanent() {
    let temp = tempfile::tempdir().unwrap();
    let path = archon_workflow::repository_record::repository_record_path(temp.path());
    std::fs::write(path, b"{").unwrap();
    assert_permanent_dispatch(
        archon_workflow::repository_record::read_repository_record(temp.path())
            .unwrap_err()
            .into(),
        "repository.lock",
    )
    .await;
}

#[tokio::test]
async fn host_command_invalid_lock_algorithm_is_permanent() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    crate::command::workflow_host_command_exec_tests::seed_frozen_subjects(
        &context,
        &[("TASK-X-001", "TASK-X-001.md")],
    );
    let path = context
        .task_root
        .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
    let mut lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    lock["algorithm"] = "invalid".into();
    std::fs::write(path, serde_json::to_vec(&lock).unwrap()).unwrap();
    let error = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        &context.project_root,
        &context.prd_path,
        &context.task_root,
    )
    .unwrap_err();
    assert_permanent_dispatch(error, "algorithm must be").await;
}

#[tokio::test]
async fn host_command_malformed_pin_is_permanent() {
    let temp = tempfile::tempdir().unwrap();
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    crate::command::workflow_host_command_exec_tests::seed_frozen_subjects(
        &context,
        &[("TASK-X-001", "TASK-X-001.md")],
    );
    let path = crate::command::workflow_task_set::acceptance_pin_path(
        &context.project_root,
        &context.task_root,
    );
    std::fs::write(path, b"{").unwrap();
    let error = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        &context.project_root,
        &context.prd_path,
        &context.task_root,
    )
    .unwrap_err();
    assert_permanent_dispatch(error, "acceptance pin").await;
}

#[tokio::test]
async fn host_command_task_set_error_lock_contention_recovers_through_dispatch() {
    let fixture = fixture(0, false);
    let context = fixture.executor.context_for_test();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(
        &context.project_root,
        &context.task_root,
    );
    let held =
        crate::command::workflow_task_set::ChainLock::acquire(&pin, &context.task_root).unwrap();
    let error = crate::command::workflow_task_set::ChainLock::acquire(&pin, &context.task_root)
        .err()
        .expect("the other holder owns the lock");
    drop(held);
    *fixture.process.evaluation.lock().unwrap() = Some(
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &error,
        ),
    );
    run(&fixture).await.unwrap();
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}

#[cfg(unix)]
struct RedactedIo(Arc<RecoveringProcess>, AtomicUsize);
#[cfg(unix)]
#[async_trait::async_trait]
impl HostCommandProcessAdapter for RedactedIo {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        if self.1.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(WorkflowError::Io {
                path: "credential-value/scratch".into(),
                source: std::io::Error::from_raw_os_error(5),
            });
        }
        self.0.execute(request, control).await
    }
}
#[cfg(unix)]
#[tokio::test]
async fn host_command_redacted_native_io_keeps_transient_class_through_dispatch() {
    let mut fixture = fixture(0, false);
    let mut context = fixture.executor.context_for_test().clone();
    context
        .freeze_provider_environment
        .insert("SERVICE_TOKEN".into(), "credential-value".into());
    let process = Arc::new(RedactedIo(fixture.process.clone(), AtomicUsize::new(0)));
    fixture.executor = Arc::new(FixedHostCommandExecutor::with_process(
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("revision")
            .unwrap(),
        context,
        fixture.store.run_dir(&fixture.run_id),
        process.clone(),
    ));
    run(&fixture).await.unwrap();
    assert_eq!(
        process.1.load(Ordering::SeqCst),
        2,
        "{:?}",
        records(&fixture)
    );
    let recorded = records(&fixture);
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].result.data["publicationReceipt"].is_object());
    assert!(
        !serde_json::to_string(&recorded)
            .unwrap()
            .contains("credential-value")
    );
    let events = std::fs::read_to_string(fixture.store.events_path(&fixture.run_id)).unwrap();
    assert!(events.contains("host_command_operational_retry"));
    assert!(!events.contains("credential-value"));
}

#[tokio::test]
async fn host_command_untyped_http_rejection_returns_once_with_original_remedy() {
    assert_permanent_dispatch(
        archon_llm::provider::LlmError::Http(
            "request rejected with status 400; correct the endpoint request field".into(),
        )
        .into(),
        "correct the endpoint request field",
    )
    .await;
}

struct BoundProcess(Arc<RecoveringProcess>);
#[async_trait::async_trait]
impl HostCommandProcessAdapter for BoundProcess {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        let mut output = self.0.execute(request.clone(), control).await?;
        let prepared: archon_workflow::PreparedPublicationV1 =
            serde_json::from_slice(&output.stdout).unwrap();
        let envelope = request
            .declared_write_set
            .iter()
            .find(|path| path.file_name().unwrap() == "gate-envelope.json")
            .unwrap();
        let root = envelope.parent().unwrap();
        let outputs = request
            .declared_write_set
            .iter()
            .filter(|path| *path != envelope)
            .map(
                |path| crate::command::workflow_gate_envelope::StagedGateOutput {
                    relative_path: path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    bytes: request.stdin.clone().expect("bound body stdin"),
                },
            )
            .collect();
        let prepared = crate::command::workflow_gate_envelope::stage_gate_evaluation(
            root,
            envelope,
            &prepared.call_id,
            &prepared.command_id,
            crate::command::workflow_gate::GateEvaluation::new("accepted", Vec::new()),
            outputs,
        )
        .unwrap();
        output.stdout = serde_json::to_vec(&prepared).unwrap();
        output.stdout_bytes = output.stdout.len() as u64;
        output.stdout_retained_bytes = output.stdout_bytes;
        Ok(output)
    }
}

#[tokio::test]
async fn host_command_binding_acquisition_preserves_transient_cause_before_admission() {
    let mut fixture = fixture(0, false);
    let context = fixture.executor.context_for_test().clone();
    fixture.executor = Arc::new(FixedHostCommandExecutor::with_process(
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("revision")
            .unwrap(),
        context.clone(),
        fixture.store.run_dir(&fixture.run_id),
        Arc::new(BoundProcess(fixture.process.clone())),
    ));
    let fault = crate::command::workflow_host_fault_test_hooks::inject(
        crate::command::workflow_host_fault_test_hooks::Point::ChainRead,
        &context.task_root,
        Box::new(|| Err(std::io::Error::from_raw_os_error(5))),
    );
    let body = std::fs::read_to_string(context.task_root.join("TASK-X-001.md")).unwrap();
    let source = format!(
        r#"async function workflow(w) {{ return await w.hostCommand("land-task-body", {{ stdin: {} }}); }}"#,
        serde_json::to_string(&body).unwrap()
    );
    run_script(&fixture, &source)
        .await
        .expect("read-only acquisition must recover");
    assert!(fault.fired());
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    assert_eq!(records(&fixture).len(), 1);
    assert!(records(&fixture)[0].result.data["publicationReceipt"].is_object());
}
