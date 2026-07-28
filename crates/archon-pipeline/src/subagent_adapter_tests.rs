use super::*;
use crate::runner::{AgentInfo, PipelineType};
use archon_llm::provider::{LlmError, LlmResponse as ProviderResponse, ModelInfo, ProviderFeature};

struct NoopClient;

#[async_trait]
impl LlmClient for NoopClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
            content: String::new(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

struct AliasProvider;

#[async_trait]
impl LlmProvider for AliasProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "gpt-5.4".into(),
            display_name: "GPT-5.4".into(),
            context_window: 1_050_000,
        }]
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>, LlmError> {
        Err(LlmError::Unsupported("not used".into()))
    }

    async fn complete(&self, _request: LlmRequest) -> Result<ProviderResponse, LlmError> {
        Err(LlmError::Unsupported("not used".into()))
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        false
    }
}

fn request(access: ToolAccessLevel) -> AgentExecutionRequest {
    AgentExecutionRequest {
        session_id: "s".into(),
        pipeline_type: PipelineType::Coding,
        task: "task".into(),
        cwd: None,
        ordinal: 1,
        attempt: 1,
        agent: AgentInfo {
            key: "context-gatherer".into(),
            display_name: "Context Gatherer".into(),
            model: "sonnet".into(),
            phase: 1,
            critical: false,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access_level: access,
        },
        messages: vec![serde_json::json!({"role":"user","content":"hello"})],
        system: vec![serde_json::json!({"type":"text","text":"system"})],
        tools: Vec::new(),
        allowed_tools: Vec::new(),
        timeout_secs: None,
        disable_auto_background: false,
        provider_env_resolution: None,
    }
}

#[test]
fn workflow_prompt_preserves_stable_system_before_volatile_run_metadata() {
    let mut first = request(ToolAccessLevel::ReadOnly);
    first.pipeline_type = PipelineType::Workflow;
    first.system = vec![serde_json::json!({
        "type":"text",
        "text":"stable workflow universe",
        "cache_control":{"type":"ephemeral"}
    })];
    first.task = "first task".into();
    first.ordinal = 1;
    let mut second = first.clone();
    second.task = "second task".into();
    second.ordinal = 2;

    let first_prompt = SubagentPipelineClient::prompt_for_request(&first);
    let second_prompt = SubagentPipelineClient::prompt_for_request(&second);

    assert_eq!(first_prompt.system, second_prompt.system);
    assert_ne!(first_prompt.prompt, second_prompt.prompt);
    assert!(!first_prompt.prompt.contains("stable workflow universe"));
    assert_eq!(first_prompt.system[0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn read_only_tools_include_memory_and_docs_but_not_writes() {
    let tools = SubagentPipelineClient::allowed_tools(&request(ToolAccessLevel::ReadOnly));
    assert!(tools.contains(&"memory_recall".to_string()));
    assert!(tools.contains(&"DocSearch".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(!tools.contains(&"Bash".to_string()));
}

#[test]
fn full_tools_include_write_and_memory_store() {
    let tools = SubagentPipelineClient::allowed_tools(&request(ToolAccessLevel::Full));
    assert!(tools.contains(&"Write".to_string()));
    assert!(tools.contains(&"memory_store".to_string()));
    assert!(tools.contains(&"ApplyPatch".to_string()));
}

#[test]
fn activity_model_resolves_tier_alias_with_active_provider() {
    let client = SubagentPipelineClient::with_provider(
        Arc::new(NoopClient),
        ToolContext::default(),
        Arc::new(AliasProvider),
    );

    assert_eq!(client.activity_model("sonnet"), "gpt-5.4");
}

#[test]
fn request_cwd_overrides_parent_context() {
    let client = SubagentPipelineClient::new(
        Arc::new(NoopClient),
        ToolContext {
            working_dir: "/project/root".into(),
            ..ToolContext::default()
        },
    );
    let mut request = request(ToolAccessLevel::Full);
    request.cwd = Some("/target/repo".into());

    assert_eq!(client.cwd_for_request(&request), "/target/repo");
}

#[test]
fn workflow_full_agent_without_bash_requests_strict_workspace_boundary() {
    let mut request = request(ToolAccessLevel::Full);
    request.pipeline_type = PipelineType::Workflow;
    request.cwd = Some("/isolated/repo".into());
    request.allowed_tools = vec!["Read".into(), "Write".into(), "Edit".into()];
    let tools = SubagentPipelineClient::allowed_tools(&request);

    assert!(SubagentPipelineClient::strict_workspace_boundary(
        &request, &tools
    ));
}

#[test]
fn workflow_command_agent_with_bash_does_not_request_strict_boundary() {
    let mut request = request(ToolAccessLevel::Full);
    request.pipeline_type = PipelineType::Workflow;
    request.cwd = Some("/target/repo".into());
    request.allowed_tools = vec!["Read".into(), "Bash".into()];
    let tools = SubagentPipelineClient::allowed_tools(&request);

    assert!(!SubagentPipelineClient::strict_workspace_boundary(
        &request, &tools
    ));
}

#[tokio::test]
async fn d47_markerless_final_gate_receives_run_scoped_provider_env() {
    let policy = ProviderEnvPolicy {
        required_keys: vec!["ARCHON_D47_FINAL_GATE_TEST".into()],
        profile_sources: Vec::new(),
        reason: Some("final gate regression".into()),
    };
    let resolution = archon_tools::provider_env::resolve_provider_env(&policy).await;
    let mut request = request(ToolAccessLevel::Full);
    request.pipeline_type = PipelineType::Workflow;
    request.disable_auto_background = true;
    request.provider_env_resolution = Some(resolution.clone());

    let source = workflow_provider_env_source(&request).expect("provider env source");
    assert_eq!(source, ProviderEnvSource::Resolution(resolution));
}
