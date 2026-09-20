//! The candidate budget belongs to the artifact, not to the provider.

use std::sync::Mutex;

use super::*;

/// Fails the first `fail_first` author calls the way a dead provider does:
/// the host turns the transport error into a failed call record, and the
/// script sees a result with no content.
struct TransportLlm {
    calls: AtomicUsize,
    fail_first: usize,
    prompts: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for TransportLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed authors must use raw run_agent")
    }

    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let ordinal = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.prompts.lock().unwrap().push(request.task);
        if ordinal <= self.fail_first {
            return Err(archon_workflow::WorkflowError::StageFailed(
                "agent transport failed: stream ended before message_stop".to_string(),
            ));
        }
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"id":"AC-X-001","ordinal":ordinal}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

struct AcceptingHost;

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for AcceptingHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "budget-{}-{}",
            request.command_id,
            archon_workflow::task_set_contract::content_digest(
                request.stdin.as_deref().unwrap_or_default().as_bytes()
            )
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
        let subjects = if request.command_id == "freeze-skeleton" {
            vec![archon_workflow::HostCommandSubject {
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010.md".into(),
            }]
        } else {
            Vec::new()
        };
        let call_id = self.call_identity(&request)?;
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
                report: serde_json::json!("accepted"),
                policy_findings: Vec::new(),
                operational_error: None,
            }),
            publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id,
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-08-27T00:00:00Z".into(),
            }),
            subjects,
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

async fn run_with(
    fail_first: usize,
) -> (
    Arc<TransportLlm>,
    Result<WorkflowV2ScriptSummary, WorkflowError>,
    String,
) {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(TransportLlm {
        calls: AtomicUsize::new(0),
        fail_first,
        prompts: Mutex::new(Vec::new()),
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(1_500),
    )
    .with_fixed_raw_tool_policy(vec!["Read".into()]);
    let runner = WorkflowV2ScriptRunner::new(
        "author budget".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        Some(serde_json::json!({
            "projectRoot": temp.path(),
            "repositoryRoot": temp.path(),
            "prdPath": temp.path().join("PRD.md"),
            "prdDigest": "a".repeat(64),
            "acceptanceCriteria": {"AC-X-001":"example criterion"},
            "taskRoot": temp.path().join("tasks"),
            "gateMode": "observe"
        })),
    )
    .with_raw_outcomes(true)
    .with_host_command_executor(Arc::new(AcceptingHost));
    let result = runner
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE)
        .await;
    let events = std::fs::read_to_string(workflow_store.events_path(&run.id)).unwrap();
    (llm, result, events)
}

#[tokio::test]
async fn a_provider_outage_does_not_spend_the_candidate_budget() {
    // A live run lost its provider mid-decomposition, burned nine of ten body
    // attempts on transport errors, and then reported the author as having
    // exhausted its attempts. A call the provider never answered says nothing
    // about the artifact, so the next real attempt is still the first one.
    let (llm, result, _events) = run_with(2).await;

    assert_eq!(result.unwrap().status, WorkflowV2Status::Accepted);
    let prompts = llm.prompts.lock().unwrap();
    let authored = &prompts[2];
    assert!(authored.contains("Logical attempt: 1."), "{authored}");
    assert!(
        !authored.contains("Provider outcome was incomplete"),
        "{authored}"
    );
}

#[tokio::test]
async fn a_dead_provider_stops_the_run_saying_so_and_stops_quickly() {
    let (llm, result, events) = run_with(usize::MAX).await;

    assert_eq!(result.unwrap().status, WorkflowV2Status::Failed);
    assert!(events.contains("agent transport failed"), "{events}");
    assert!(!events.contains("exhausted"), "{events}");
    // The acceptance phase alone allows six candidate attempts; an outage must
    // stop well before spending them.
    let calls = llm.calls.load(Ordering::SeqCst);
    assert!(calls <= 3, "{calls}");
}
