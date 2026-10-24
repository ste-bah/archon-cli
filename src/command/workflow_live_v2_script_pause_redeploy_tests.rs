use super::workflow_live_v2_script_pause_tests::{new_run, resume, runner};
use super::*;

struct BuildBoundExecutor {
    build: AtomicUsize,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor
    for BuildBoundExecutor
{
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "slot-{}-{}",
            request.command_id,
            self.build.load(Ordering::SeqCst)
        ))
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
        _expected_generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: "passed".into(),
            stderr: String::new(),
            stdout_bytes: 6,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!({ "command": request.command_id, "passed": true }),
                policy_findings: Vec::new(),
                operational_error: None,
            }),
            publication_receipt: None,
            subjects: Vec::new(),
            postcondition: None,
            observed_only: false,
        })
    }
}

#[tokio::test]
async fn a_new_build_passes_the_pause_after_only_its_derived_slot_reexecutes() {
    let (_temp, store, run_id) = new_run();
    let executor = Arc::new(BuildBoundExecutor {
        build: AtomicUsize::new(1),
        calls: AtomicUsize::new(0),
    });
    let command = crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
        .iter()
        .find(|logic| {
            logic.build_bound
                && logic
                    .sources
                    .iter()
                    .flat_map(|group| group.iter())
                    .any(|source| source.ends_with("workflow_decompose_frozen_chain.rs"))
        })
        .expect("a build-bound capability")
        .id;
    let script = format!(
        r#"async function workflow(w) {{
            await w.hostCommand("{command}", {{ stdin: "same-input" }});
            await w.pause("pause-body-TASK-X-001-1", {{ subject: "TASK-X-001", reason: "no_progress" }});
            return {{ continued: true }};
        }}"#
    );
    let (first, _first_rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(executor.clone()),
        None,
    );
    let error = first.run(&script).await.expect_err("the first call pauses");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );

    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let old_call_id = format!("slot-{command}-1");
    assert!(v2.load_call_record(&old_call_id).unwrap().is_some());
    std::fs::remove_file(v2.result_path(&old_call_id))
        .expect("the new build replaces the old slot");
    executor.build.store(2, Ordering::SeqCst);
    resume(&store, &run_id);

    let (second, _second_rx) = runner(
        &store,
        &run_id,
        Arc::new(PanicLlm),
        Some(executor.clone()),
        None,
    );
    let summary = second
        .run(&script)
        .await
        .expect("the resume passes the old pause");

    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
}
