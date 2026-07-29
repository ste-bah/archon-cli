use super::*;
use archon_workflow::{WorkflowV2HostCall, WorkflowV2HostMethod};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct RecordingClient {
    last_request: Mutex<Option<AgentExecutionRequest>>,
    requests: Mutex<Vec<AgentExecutionRequest>>,
}

#[async_trait::async_trait]
impl LlmClient for RecordingClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        Ok(archon_pipeline::runner::LlmResponse {
            content: "fallback".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }

    async fn run_agent(
        &self,
        request: AgentExecutionRequest,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        self.requests
            .lock()
            .expect("requests lock")
            .push(request.clone());
        *self.last_request.lock().expect("recording lock") = Some(request);
        Ok(archon_pipeline::runner::LlmResponse {
            content: "recorded".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

fn request(
    method: WorkflowV2HostMethod,
    write_mode: Option<WorkflowV2WriteMode>,
) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "discover".to_string(),
            method,
            write_mode,
            options: Default::default(),
        },
        role: if write_mode.is_some() {
            "coder"
        } else {
            "researcher"
        }
        .to_string(),
        task: "inspect repository and task files".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({ "objective": "test" }),
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

#[tokio::test]
async fn generated_v2_request_keeps_stable_prompt_in_system_context() {
    let recorder = Arc::new(RecordingClient::default());
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        recorder.clone(),
        tui_tx,
        Vec::new(),
        "wf-test".to_string(),
        Some("/repo".to_string()),
        Some(17),
    );
    let mut request = request(WorkflowV2HostMethod::Agent, None);
    request.input = serde_json::json!({
        "task_universe":{
            "schema_version":"workflow-v2-task-universe-v1",
            "source_roots":["project-tasks"],
            "tasks":[{
                "canonical_task_id":"TASK-1",
                "description":"stable universe"
            }]
        },
        "wave":1
    });
    let prompt = archon_workflow::WorkflowV2AgentAdapter::new().build_prompt_parts(&request);

    client
        .run_agent_request(&request, prompt.invocation.clone())
        .await
        .expect("recorded request");

    let recorded = recorder
        .last_request
        .lock()
        .expect("recording lock")
        .clone()
        .expect("recorded request");
    let system = recorded.system[0]["text"].as_str().expect("system text");
    let message = recorded.messages[0]["content"]
        .as_str()
        .expect("message text");
    assert!(system.contains(&prompt.stable_prefix));
    assert!(!message.contains("stable universe"));
    assert!(message.contains("call_id: discover"));
}

#[test]
fn read_only_v2_agent_gets_repository_cwd_and_read_tools() {
    let stage = stage_request_for_v2_agent(
        "wf-test",
        ProviderTier::Researcher,
        None,
        &request(WorkflowV2HostMethod::Agent, None),
    );

    assert_eq!(stage.stage_kind, StageKind::Agent);
    assert_eq!(
        request_target_repository_root(&stage),
        Some(PathBuf::from("/repo"))
    );
    let tools = allowed_tools(&stage);
    assert!(tools.contains(&"Read".to_string()));
    assert!(tools.contains(&"Grep".to_string()));
    assert!(tools.contains(&"Glob".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(!tools.contains(&"Bash".to_string()));
}

#[test]
fn generated_v2_verification_fanout_gets_bash_without_write_tools() {
    let mut req = request(WorkflowV2HostMethod::Parallel, None);
    req.call.id = "verification-wave-1".to_string();
    req.input = serde_json::json!({
        "focused_verification": [
            {"command": "cargo test -p archon-trading data_store -- --nocapture"}
        ]
    });
    let stage = stage_request_for_v2_agent("wf-test", ProviderTier::Coder, None, &req);

    let tools = allowed_tools(&stage);

    assert!(tools.contains(&"Bash".to_string()));
    assert!(tools.contains(&"Read".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
}

#[test]
fn generated_v2_verification_resolves_project_artifact_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("project/.archon/workflows/wf-test/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    let mut req = request(WorkflowV2HostMethod::Parallel, None);
    req.call.id = "verification-wave-1".to_string();
    req.project_artifacts = archon_workflow::project_artifact_context_from_v2_root(&v2_root);
    req.input = serde_json::json!({
        "artifact_requirements": [
            ".archon/workflows/wf-test/artifacts/data-lake-gap-audit.json",
            {"path": ".archon/trading-lab/data/registry.json"}
        ],
        "focused_verification": "check project artifacts"
    });

    let stage = stage_request_for_v2_agent("wf-test", ProviderTier::Coder, None, &req);
    let paths = stage
        .input
        .get("project_artifact_paths")
        .and_then(serde_json::Value::as_array)
        .expect("resolved project artifact paths");

    assert_eq!(paths.len(), 2);
    assert!(paths.iter().any(|entry| {
        entry
            .get("absolute_path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| {
                path.ends_with(".archon/workflows/wf-test/artifacts/data-lake-gap-audit.json")
            })
    }));
    assert!(!allowed_tools(&stage).contains(&"Write".to_string()));
}

#[test]
fn verification_input_stamps_project_relative_artifacts_into_canonical_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path().join("project");
    let v2_root = project_root.join(".archon/workflows/wf-test/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    for index in 1..=5 {
        let validation = project_root.join(format!(
            ".archon/trading-lab/data/datasets/dataset-{index}/v1/validation.json"
        ));
        std::fs::create_dir_all(validation.parent().expect("validation parent"))
            .expect("dataset directory");
        std::fs::write(validation, "{}\n").expect("validation artifact");
    }
    let fixture = include_str!("fixtures/wf346_verification_project_relative_item.json");
    let mut req = request(WorkflowV2HostMethod::Parallel, None);
    req.call.id = "verification-wave-2".to_string();
    req.project_artifacts = archon_workflow::project_artifact_context_from_v2_root(&v2_root);
    req.input = serde_json::from_str(fixture).expect("verification fixture");

    let stage = stage_request_for_v2_agent("wf-test", ProviderTier::Coder, None, &req);
    let dataset_root = project_root.join(".archon/trading-lab/data/datasets");
    let registry = project_root.join(".archon/trading-lab/data/registry.json");
    let requirements = stage.input["artifact_requirements"]
        .as_array()
        .expect("artifact requirements");
    let focused = stage.input["focused_verification"][0]
        .as_str()
        .expect("focused verification");

    assert_eq!(requirements[0], dataset_root.display().to_string());
    assert_eq!(requirements[1], registry.display().to_string());
    assert!(focused.contains(&dataset_root.display().to_string()));
    assert!(focused.contains(&registry.display().to_string()));
    assert!(!focused.contains("Inspect .archon/"));
    assert_eq!(
        std::fs::read_dir(dataset_root)
            .expect("resolved dataset root")
            .count(),
        5
    );
}

#[test]
fn write_capable_v2_agent_gets_full_tools_and_ownership_metadata() {
    let stage = stage_request_for_v2_agent(
        "wf-test",
        ProviderTier::Coder,
        Some("/fallback".to_string()),
        &request(
            WorkflowV2HostMethod::Implementation,
            Some(WorkflowV2WriteMode::Coordinated),
        ),
    );

    assert_eq!(stage.stage_kind, StageKind::Implementation);
    assert_eq!(
        request_target_repository_root(&stage),
        Some(PathBuf::from("/repo"))
    );
    assert_eq!(
        stage
            .input
            .get("write_coordination")
            .and_then(|value| value.get("enabled"))
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    let tools = allowed_tools(&stage);
    assert!(tools.contains(&"Write".to_string()));
    assert!(tools.contains(&"Edit".to_string()));
    assert!(tools.contains(&"ApplyPatch".to_string()));
}

#[test]
fn generated_v2_write_with_project_artifacts_gets_bash_and_project_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("runtime/.archon/workflows/wf-test/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    let mut req = request(
        WorkflowV2HostMethod::Fanout,
        Some(WorkflowV2WriteMode::Coordinated),
    );
    req.call.id = "remediation-wave-1".to_string();
    req.project_artifacts = archon_workflow::project_artifact_context_from_v2_root(&v2_root);
    req.input = serde_json::json!({
        "artifact_requirements": [
            {"path": ".archon/trading-lab/data/registry-migration-report.json"}
        ]
    });

    let stage = stage_request_for_v2_agent("wf-test", ProviderTier::Coder, None, &req);
    let tools = allowed_tools(&stage);

    assert!(tools.contains(&"Bash".to_string()));
    assert_eq!(
        stage
            .input
            .get("project_artifact_root")
            .and_then(serde_json::Value::as_str),
        req.project_artifacts.project_root.as_deref()
    );
    assert_eq!(
        stage
            .input
            .get("workflow_branch_evidence_root")
            .and_then(serde_json::Value::as_str),
        req.project_artifacts.branch_evidence_root.as_deref()
    );
    assert_eq!(
        stage
            .input
            .get("stage_extra")
            .and_then(|value| value.get("required_tools"))
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn worktree_v2_agent_marks_write_coordination_enabled() {
    let stage = stage_request_for_v2_agent(
        "wf-test",
        ProviderTier::Coder,
        None,
        &request(
            WorkflowV2HostMethod::Implementation,
            Some(WorkflowV2WriteMode::Worktree),
        ),
    );

    assert_eq!(
        stage
            .input
            .get("write_coordination")
            .and_then(|value| value.get("enabled"))
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        stage
            .input
            .get("write_coordination")
            .and_then(|value| value.get("mode"))
            .and_then(serde_json::Value::as_str),
        Some("worktree")
    );
}

#[test]
fn read_only_fanout_parallelism_clamps_to_live_subagent_cap() {
    assert_eq!(read_only_v2_fanout_parallelism(Some(8), Some(4)), 4);
    assert_eq!(read_only_v2_fanout_parallelism(Some(2), Some(4)), 2);
    assert_eq!(read_only_v2_fanout_parallelism(None, Some(4)), 4);
    assert_eq!(read_only_v2_fanout_parallelism(Some(0), Some(4)), 1);
}

#[test]
fn read_only_fanout_parallelism_uses_default_subagent_cap_when_executor_missing() {
    assert_eq!(read_only_v2_fanout_parallelism(Some(8), None), 4);
    assert_eq!(read_only_v2_fanout_parallelism(None, None), 4);
}

#[tokio::test]
async fn closed_tui_prevents_v2_agent_launch() {
    let recorder = Arc::new(RecordingClient::default());
    let (tui_tx, tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    drop(tui_rx);
    let client = LiveV2AgentClient::new(
        recorder.clone(),
        tui_tx,
        Vec::new(),
        "wf-test".to_string(),
        Some("/repo".to_string()),
        Some(17),
    );

    client
        .run_agent_request(
            &request(WorkflowV2HostMethod::Agent, None),
            "inspect".to_string(),
        )
        .await
        .expect_err("closed TUI must prevent V2 agent launch");

    assert!(recorder.requests.lock().expect("requests lock").is_empty());
}

#[tokio::test]
async fn generated_v2_agent_requests_are_foreground_with_configured_timeout() {
    let recorder = Arc::new(RecordingClient::default());
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        recorder.clone(),
        tui_tx,
        Vec::new(),
        "wf-test".to_string(),
        Some("/repo".to_string()),
        Some(17),
    );

    let result = client
        .run_agent_request(
            &request(WorkflowV2HostMethod::Agent, None),
            "inspect".to_string(),
        )
        .await
        .expect("agent response");

    assert_eq!(result, "recorded");
    let recorded = recorder
        .last_request
        .lock()
        .expect("recording lock")
        .clone()
        .expect("recorded request");
    assert_eq!(recorded.pipeline_type, PipelineType::Workflow);
    assert_eq!(recorded.timeout_secs, Some(17));
    assert!(
        recorded.disable_auto_background,
        "generated V2 awaited host calls must remain foreground-controlled"
    );
}

#[tokio::test]
async fn d47_one_run_uses_identical_provider_presence_for_subagents_and_final_gate() {
    let profile_dir = tempfile::tempdir().expect("profile dir");
    let profile = profile_dir.path().join("profile");
    std::fs::write(&profile, "export ARCHON_DEMO_PROVIDER_KEY=present-value\n").expect("profile");
    let policy = archon_tools::provider_env::ProviderEnvPolicy {
        required_keys: vec!["ARCHON_DEMO_PROVIDER_KEY".to_string()],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("run-scoped invariant".to_string()),
    };
    let resolution = archon_tools::provider_env::resolve_provider_env(&policy).await;
    let expected_proof = resolution.proof.clone();
    let recorder = Arc::new(RecordingClient::default());
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        recorder.clone(),
        tui_tx,
        Vec::new(),
        "wf-provider-invariant".to_string(),
        Some("/repo".to_string()),
        Some(17),
    )
    .with_provider_env_resolution(Some(resolution));

    let mut implementation = request(
        WorkflowV2HostMethod::Implementation,
        Some(WorkflowV2WriteMode::Coordinated),
    );
    implementation.call.id = "implementation-wave-1".to_string();
    let mut verification = request(WorkflowV2HostMethod::Parallel, None);
    verification.call.id = "verification-wave-1".to_string();
    let mut final_gate = request(WorkflowV2HostMethod::Agent, None);
    final_gate.call.id = "final-zero-gap-audit".to_string();

    for request in [&implementation, &verification, &final_gate] {
        client
            .run_agent_request(request, request.task.clone())
            .await
            .expect("recorded workflow request");
    }

    let recorded = recorder.requests.lock().expect("requests lock");
    assert_eq!(recorded.len(), 3);
    for request in recorded.iter() {
        let proof = &request
            .provider_env_resolution
            .as_ref()
            .expect("run-scoped provider resolution")
            .proof;
        assert_eq!(proof, &expected_proof);
    }
    assert!(!format!("{expected_proof:?}").contains("present-value"));
}
