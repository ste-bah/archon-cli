//! #297 round 9: a host command whose staging could not be sealed after its
//! publication was committed pauses the run and returns its receipt; the
//! script path records the call as completed, so a resume reuses it and
//! never publishes again.
use super::*;
use crate::command::workflow_host_staging_pause::StagingPause;

/// The executor's post-commit outcome (proved against the real executor in
/// `workflow_host_boundary_r9_tests`): the run paused by the staging pause,
/// the residue recorded, and the committed result returned.
struct SealFailsAfterCommit {
    project: std::path::PathBuf,
    run_root: std::path::PathBuf,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor
    for SealFailsAfterCommit
{
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!("host-command:{}:fixed", request.command_id))
    }

    fn record_is_reusable(
        &self,
        _record: &archon_workflow::WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(true)
    }

    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let call_id = self.call_identity(&request)?;
        let generation = match expected_generation {
            Some(generation) => generation,
            None => {
                WorkflowStore::project(&self.project)
                    .load_state(self.run_root.file_name().unwrap().to_str().unwrap())?
                    .generation
            }
        };
        let pause = StagingPause::new(
            &self.project,
            &self.run_root,
            generation,
            &call_id,
            &request.command_id,
        )?;
        let root = self.run_root.join("host-command-staging").join("call");
        let paused = pause.pause(
            &root,
            "staging could not be sealed (it was removed)",
            "fixture",
            false,
        );
        assert!(
            matches!(paused, WorkflowError::ControlPaused(_)),
            "{paused:?}"
        );
        pause.record_residue(
            &root,
            "staging could not be sealed after the publication was committed",
            "fixture",
        );
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: "published".into(),
            stderr: String::new(),
            stdout_bytes: 9,
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
                call_id,
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-10-08T00:00:00Z".into(),
            }),
            subjects: Vec::new(),
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

#[tokio::test]
async fn a_post_commit_seal_failure_records_the_call_and_pauses_the_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::project(temp.path());
    let mut run = workflow_store.create_run(spec.clone()).expect("run");
    run.status = archon_workflow::RunStatus::Running;
    workflow_store.save_state(&run).expect("running");
    let run_root = workflow_store.run_dir(&run.id);
    let v2_store = WorkflowV2ResultStore::new(run_root.join("v2"));
    let (ui_sink, _tui_rx) = default_workflow_ui_sink();
    let executor = Arc::new(SealFailsAfterCommit {
        project: temp.path().to_path_buf(),
        run_root,
        calls: AtomicUsize::new(0),
    });
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let outcome = WorkflowV2ScriptRunner::new(
        "post-commit seal failure".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    )
    .with_host_command_executor(executor.clone())
    .run(r#"async function workflow(w) { return await w.hostCommand("task-set-lint", { stdin: null }); }"#)
    .await;
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let state = workflow_store.load_state(&run.id).expect("state");
    assert_eq!(
        state.status,
        archon_workflow::RunStatus::Paused,
        "{outcome:?}"
    );
    let record = v2_store
        .load_call_record("host-command:task-set-lint:fixed")
        .expect("record lookup")
        .expect("the completed call was not recorded: a resume would publish again");
    assert_eq!(record.status, WorkflowV2Status::Accepted, "{outcome:?}");
    assert!(
        !record.result.data["publicationReceipt"].is_null(),
        "the receipt was lost: {}",
        record.result.data
    );
}
