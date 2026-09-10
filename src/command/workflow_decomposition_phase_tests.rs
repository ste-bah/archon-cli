//! Phase-order proof for the embedded fixed-decomposition script.

use std::sync::Mutex;

use super::*;

struct PhaseLlm {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for PhaseLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed phase authors must use raw run_agent")
    }

    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let ordinal = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"id":"AC-X-001","ordinal":ordinal}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

struct PhaseHost {
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for PhaseHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(format!(
            "phase-{}-{}",
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
        self.calls.lock().unwrap().push(request.command_id.clone());
        let subjects = if request.command_id == "freeze-skeleton" {
            vec![
                archon_workflow::HostCommandSubject {
                    task_id: "TASK-X-010".into(),
                    file_name: "TASK-X-010.md".into(),
                },
                archon_workflow::HostCommandSubject {
                    task_id: "TASK-X-020".into(),
                    file_name: "TASK-X-020.md".into(),
                },
            ]
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

#[tokio::test]
async fn embedded_script_executes_phase_zero_a_b_all_c_d_e_in_order() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(PhaseLlm {
        calls: AtomicUsize::new(0),
    });
    let host = Arc::new(PhaseHost {
        calls: Mutex::new(Vec::new()),
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(1_500),
    )
    .with_fixed_raw_tool_policy(vec![
        "Read".into(),
        "Grep".into(),
        "Glob".into(),
        "CartographerScan".into(),
    ]);
    let runner = WorkflowV2ScriptRunner::new(
        "fixed decomposition".into(),
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
            "prdPath": temp.path().join("PRD.md"),
            "prdDigest": "a".repeat(64),
            "acceptanceCriteria": {"AC-X-001":"example criterion"},
            "taskRoot": temp.path().join("tasks"),
            "gateMode": "observe"
        })),
    )
    .with_raw_outcomes(true)
    .with_host_command_executor(host.clone());

    let summary = runner
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE)
        .await
        .unwrap();

    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        *host.calls.lock().unwrap(),
        [
            "freeze-acceptance",
            "freeze-skeleton",
            "land-task-body",
            "land-task-body",
            "task-set-lint",
            "requirements-trace",
        ]
    );
}

struct RetryLlm {
    calls: AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for RetryLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        panic!("fixed retry authors must use raw run_agent")
    }

    async fn run_agent(
        &self,
        request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(request.task);
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"id":"AC-X-001"}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

struct RetryHost {
    calls: Mutex<Vec<String>>,
    acceptance_attempts: AtomicUsize,
    first_scope: archon_workflow::RemediationScope,
    first_has_receipt: bool,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for RetryHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        let ordinal = if request.command_id == "freeze-acceptance" {
            self.acceptance_attempts.load(Ordering::SeqCst) + 1
        } else {
            self.calls.lock().unwrap().len() + 1
        };
        Ok(format!("retry-{}-{ordinal}", request.command_id))
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
        self.calls.lock().unwrap().push(request.command_id.clone());
        let acceptance_attempt = if request.command_id == "freeze-acceptance" {
            self.acceptance_attempts.fetch_add(1, Ordering::SeqCst) + 1
        } else {
            0
        };
        let first = acceptance_attempt == 1;
        let findings = if first {
            vec![archon_workflow::GatePolicyFinding {
                text: "exact authoritative correction".into(),
                subject: "acceptance".into(),
                source_path: None,
                remediation_scope: self.first_scope,
            }]
        } else {
            Vec::new()
        };
        let has_receipt = !first || self.first_has_receipt;
        let subjects = if request.command_id == "freeze-skeleton" {
            vec![archon_workflow::HostCommandSubject {
                task_id: "TASK-X-010".into(),
                file_name: "TASK-X-010.md".into(),
            }]
        } else {
            Vec::new()
        };
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
                report: serde_json::json!("fixture"),
                policy_findings: findings,
                operational_error: None,
            }),
            publication_receipt: has_receipt.then(|| archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id: format!("receipt-{}-{acceptance_attempt}", request.command_id),
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-08-27T00:00:00Z".into(),
            }),
            subjects,
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: has_receipt,
                summary: "fixture".into(),
            }),
        })
    }
}

async fn run_retry_fixture(
    scope: archon_workflow::RemediationScope,
    first_has_receipt: bool,
) -> (
    Arc<RetryLlm>,
    Arc<RetryHost>,
    Result<WorkflowV2ScriptSummary, WorkflowError>,
    String,
) {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (ui_sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(RetryLlm {
        calls: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    });
    let host = Arc::new(RetryHost {
        calls: Mutex::new(Vec::new()),
        acceptance_attempts: AtomicUsize::new(0),
        first_scope: scope,
        first_has_receipt,
    });
    let client = LiveV2AgentClient::new(
        llm.clone(),
        ui_sink,
        Vec::new(),
        run.id.clone(),
        None,
        Some(1_500),
    )
    .with_fixed_raw_tool_policy(vec![
        "Read".into(),
        "Grep".into(),
        "Glob".into(),
        "CartographerScan".into(),
    ]);
    let runner = WorkflowV2ScriptRunner::new(
        "retry fixture".into(),
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
            "prdPath": temp.path().join("PRD.md"),
            "prdDigest": "a".repeat(64),
            "acceptanceCriteria": {"AC-X-001":"example criterion"},
            "taskRoot": temp.path().join("tasks"),
            "gateMode": "observe"
        })),
    )
    .with_raw_outcomes(true)
    .with_host_command_executor(host.clone());
    let result = runner
        .run(crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE)
        .await;
    let events = std::fs::read_to_string(workflow_store.events_path(&run.id)).unwrap();
    (llm, host, result, events)
}

#[tokio::test]
async fn candidate_finding_retries_and_feeds_exact_text_to_responsible_author() {
    let (llm, host, result, _events) =
        run_retry_fixture(archon_workflow::RemediationScope::CandidateArtifact, false).await;

    assert_eq!(result.unwrap().status, WorkflowV2Status::Accepted);
    assert_eq!(host.acceptance_attempts.load(Ordering::SeqCst), 2);
    let prompts = llm.prompts.lock().unwrap();
    assert!(
        prompts[1].contains("exact authoritative correction"),
        "{}",
        prompts[1]
    );
}

#[tokio::test]
async fn prd_input_finding_stops_before_skeleton_or_body_dispatch() {
    let (_llm, host, result, events) =
        run_retry_fixture(archon_workflow::RemediationScope::PrdInput, false).await;

    assert_eq!(result.unwrap().status, WorkflowV2Status::Failed);
    assert!(
        events.contains("exact authoritative correction"),
        "{events}"
    );
    assert_eq!(*host.calls.lock().unwrap(), ["freeze-acceptance"]);
}

#[tokio::test]
async fn acceptance_without_committed_receipt_cannot_start_skeleton() {
    let (_llm, host, result, events) = run_retry_fixture(
        archon_workflow::RemediationScope::InheritedPredecessor,
        false,
    )
    .await;

    assert_eq!(result.unwrap().status, WorkflowV2Status::Failed);
    assert!(
        events.contains("no committed publication receipt"),
        "{events}"
    );
    assert_eq!(*host.calls.lock().unwrap(), ["freeze-acceptance"]);
}

#[tokio::test]
async fn the_skeleton_author_is_shown_the_shape_of_a_dependency_entry() {
    // `depends_on: []` teaches an author nothing about what an entry holds. A
    // live run guessed a bare id string, then a map of the wrong shape, then
    // corrupted a task id, and failed with its attempts spent.
    let (llm, _host, result, _events) =
        run_retry_fixture(archon_workflow::RemediationScope::CandidateArtifact, false).await;
    result.unwrap();

    let prompts = llm.prompts.lock().unwrap();
    let prompt = prompts
        .iter()
        .find(|prompt| prompt.contains("\"file_name\""))
        .expect("a skeleton author prompt");
    let start = prompt
        .find("{\"schema_version\":1,\"acceptance_digest\"")
        .expect("the skeleton shape template");
    let shape: serde_json::Value = serde_json::Deserializer::from_str(&prompt[start..])
        .into_iter()
        .next()
        .expect("one shape document")
        .expect("the shape template parses as JSON");

    let dependency = &shape["tasks"][0]["depends_on"][0];
    assert!(
        dependency["task_id"].is_string(),
        "a dependency entry must show its task_id: {shape}"
    );
}
