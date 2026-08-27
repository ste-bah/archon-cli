use super::*;

struct OrderingExecutor;

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for OrderingExecutor {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:ordering", request.command_id))
    }

    fn record_is_reusable(
        &self,
        _record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!("passed"),
                policy_findings: Vec::new(),
                operational_error: None,
            }),
            publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id: format!("host-command:{}:ordering", request.command_id),
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-08-27T00:00:00Z".into(),
            }),
            subjects: Vec::new(),
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

struct DurableOrderingSink {
    store: WorkflowStore,
    run_id: String,
    log_path: std::path::PathBuf,
    activity_count: AtomicUsize,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowUiSink for DurableOrderingSink {
    async fn emit(
        &self,
        event: archon_workflow::WorkflowUiEvent,
    ) -> archon_workflow::WorkflowUiResult {
        if let archon_workflow::WorkflowUiEvent::Activity(update) = event {
            let events = std::fs::read_to_string(self.store.events_path(&self.run_id))
                .expect("durable event exists before transient update");
            let log = std::fs::read_to_string(&self.log_path)
                .expect("flushed decomposition log exists before transient update");
            let detail = update.detail.as_deref().unwrap_or_default();
            if detail.contains("host_command_started") {
                assert!(events.contains("host_command_started"), "{events}");
                assert!(log.contains("status=running"), "{log}");
            } else {
                assert!(
                    detail.contains("host_command_completed")
                        || detail.contains("subject_accepted"),
                    "{detail}"
                );
                assert!(
                    events.contains("host_command_completed")
                        || events.contains("subject_accepted"),
                    "{events}"
                );
                assert!(log.contains("status=accepted"), "{log}");
            }
            assert!(log.contains("phase=set_gates"), "{log}");
            self.activity_count.fetch_add(1, Ordering::SeqCst);
        } else if let archon_workflow::WorkflowUiEvent::Text(text) = event {
            assert!(
                !text.contains("Workflow V2 script call running"),
                "fixed pre-persistence progress leaked: {text}"
            );
        }
        Ok(())
    }
}

pub(super) fn seed_fixed_progress_state(
    store: &WorkflowStore,
    run_id: &str,
    log_path: &std::path::Path,
) {
    store
        .write_run_json(
            run_id,
            crate::command::workflow_decompose_state::FIXED_STATE_PATH,
            &archon_workflow::FixedDecompositionStateV1 {
                schema_version: archon_workflow::FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
                run_kind: archon_workflow::WorkflowRunKind::FixedDecompositionV1,
                identity: archon_workflow::FixedRunIdentityV1 {
                    template_version: "fixed-decomposition-v1".into(),
                    starting_binary_revision: "rev".into(),
                    script_digest: "script".into(),
                    catalog_digest: "catalog".into(),
                    project_root_identity: "/project".into(),
                    prd_identity: "/project/PRD.md".into(),
                    task_root_identity: "/project/tasks".into(),
                },
                phase: archon_workflow::DecompositionPhase::Identity,
                attempts: Default::default(),
                dispositions: Default::default(),
                log_path: log_path.display().to_string(),
            },
        )
        .unwrap();
}

#[tokio::test]
async fn fixed_progress_emits_only_after_durable_event_and_log_flush() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let log_path = temp.path().join("tasks/.decompose.log");
    seed_fixed_progress_state(&workflow_store, &run.id, &log_path);
    let sink = Arc::new(DurableOrderingSink {
        store: workflow_store.clone(),
        run_id: run.id.clone(),
        log_path,
        activity_count: AtomicUsize::new(0),
    });
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        sink.clone(),
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    WorkflowV2ScriptRunner::new(
        "fixed progress ordering".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store,
        run.id,
        true,
        None,
        None,
    )
    .with_host_command_executor(Arc::new(OrderingExecutor))
    .run(r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#)
    .await
    .unwrap();

    assert_eq!(sink.activity_count.load(Ordering::SeqCst), 2);
}
