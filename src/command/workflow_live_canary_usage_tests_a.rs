use super::*;

pub(super) const CANARY_TEST: &str = "command::workflow_live::workflow_live_canary_tests::usage_tests::canary_wf_afae6bee_provider_ledger";
pub(super) const CANARY_CHILD_ENV: &str = "ARCHON_CANARY_USAGE_CHILD";
const CANARY_USAGE_SCOPE: &str = "wf-afae6bee-measurement";

#[derive(Clone)]
pub(super) struct ScopedCanaryClient {
    inner: Arc<dyn LlmClient>,
}

impl ScopedCanaryClient {
    pub(super) fn new(inner: Arc<dyn LlmClient>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl LlmClient for ScopedCanaryClient {
    fn provider_id(&self) -> Option<String> {
        self.inner.provider_id()
    }

    fn resolve_model_alias(&self, model: &str) -> String {
        self.inner.resolve_model_alias(model)
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        let request = archon_pipeline::runner::AgentExecutionRequest {
            pipeline_type: archon_pipeline::runner::PipelineType::Workflow,
            session_id: CANARY_USAGE_SCOPE.into(),
            cwd: None,
            task: "controlled canary planner call".into(),
            ordinal: 0,
            attempt: 1,
            agent: archon_pipeline::runner::AgentInfo {
                key: "planner".into(),
                display_name: "Planner".into(),
                model: model.into(),
                phase: 0,
                critical: true,
                parallelizable: false,
                quality_threshold: 0.0,
                tool_access_level: archon_pipeline::runner::ToolAccessLevel::ReadOnly,
            },
            messages,
            system,
            tools,
            allowed_tools: Vec::new(),
            timeout_secs: None,
            disable_auto_background: true,
            provider_env_resolution: None,
        };
        self.inner.run_agent(request).await
    }

    async fn run_agent(
        &self,
        mut request: archon_pipeline::runner::AgentExecutionRequest,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        request.session_id = CANARY_USAGE_SCOPE.into();
        self.inner.run_agent(request).await
    }
}

pub(super) struct CanaryProvider {
    script: Arc<CanaryAgentClient>,
    request_bytes: Arc<Mutex<Vec<u64>>>,
}

impl CanaryProvider {
    pub(super) fn new(script: Arc<CanaryAgentClient>, request_bytes: Arc<Mutex<Vec<u64>>>) -> Self {
        Self {
            script,
            request_bytes,
        }
    }

    fn response_for_request(&self, request: &LlmRequest) -> (String, u64) {
        let mut prompt = String::new();
        for value in request.system.iter().chain(request.messages.iter()) {
            collect_text(value, &mut prompt);
        }
        let content = self.script.respond(&prompt);
        self.script
            .prompts
            .lock()
            .expect("prompt log lock")
            .push(prompt.chars().take(2000).collect());
        let bytes = measured_request_bytes(request);
        self.request_bytes
            .lock()
            .expect("request byte log lock")
            .push(bytes);
        (content, bytes)
    }
}

#[async_trait]
impl LlmProvider for CanaryProvider {
    fn name(&self) -> &str {
        "controlled-canary"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "controlled-canary-model".into(),
            display_name: "Controlled Canary Model".into(),
            context_window: 200_000,
        }]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let (content, request_bytes) = self.response_for_request(&request);
        let input_tokens = request_bytes.div_ceil(4);
        let output_tokens = content.len().div_ceil(4) as u64;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            for event in provider_events(content, input_tokens, output_tokens) {
                tx.send(event).await.expect("send canary stream event");
            }
        });
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<ProviderResponse, LlmError> {
        Err(LlmError::Unsupported(
            "controlled canary uses streaming".into(),
        ))
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
}

fn measured_request_bytes(request: &LlmRequest) -> u64 {
    serde_json::to_vec(&serde_json::json!({
        "system": request.system,
        "messages": request.messages,
        "tools": request.tools,
    }))
    .expect("serialize measured provider request")
    .len() as u64
}

fn provider_events(content: String, input_tokens: u64, output_tokens: u64) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: uuid::Uuid::new_v4().to_string(),
            model: "controlled-canary-model".into(),
            usage: Usage {
                input_tokens,
                input_tokens_available: true,
                cache_creation_input_tokens_available: true,
                cache_read_input_tokens_available: true,
                ..Usage::default()
            },
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: content,
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Some(Usage {
                output_tokens,
                output_tokens_available: true,
                ..Usage::default()
            }),
        },
        StreamEvent::MessageStop,
    ]
}

fn collect_text(value: &serde_json::Value, into: &mut String) {
    match value {
        serde_json::Value::String(text) => {
            into.push_str(text);
            into.push('\n');
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, into);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_text(item, into);
            }
        }
        _ => {}
    }
}

pub(super) fn install_canary_executor(provider: Arc<dyn LlmProvider>, root: &std::path::Path) {
    let agent_config = AgentConfig {
        session_id: CANARY_USAGE_SCOPE.into(),
        working_dir: root.to_path_buf(),
        ..AgentConfig::default()
    };
    let executor = AgentSubagentExecutor::new(
        provider,
        create_default_registry(root.to_path_buf(), None),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(root))),
        None,
        None,
        root.to_path_buf(),
        CANARY_USAGE_SCOPE.into(),
        "controlled-canary-model".into(),
        Vec::new(),
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(None)),
        Arc::new(agent_config),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            CANARY_USAGE_SCOPE.into(),
            String::new(),
            String::new(),
        )),
    );
    install_subagent_executor(Arc::new(executor));
}

pub(super) fn assert_canary_usage(path: &std::path::Path, expected_rows: usize, request_bytes: &[u64]) {
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().expect("UTF-8 learning path"),
        "reopen canary learning db",
    )
    .expect("learning db");
    let rows = list_llm_call_usage(
        &db,
        &LlmCallUsageScope::new(Some(CANARY_USAGE_SCOPE), Some(CANARY_USAGE_SCOPE)),
    )
    .expect("list canary usage");
    assert_eq!(
        rows.len(),
        expected_rows,
        "one durable row per provider call"
    );
    assert_eq!(
        rows.len(),
        request_bytes.len(),
        "ledger and provider call counts differ"
    );
    assert!(
        rows.iter().all(|row| {
            row.provider_id == "controlled-canary"
                && row.model_id == "controlled-canary-model"
                && row.terminal_status == "succeeded"
                && matches!(row.input_tokens, UsageAvailability::Known(_))
                && matches!(row.output_tokens, UsageAvailability::Known(_))
                && matches!(row.cache_creation_input_tokens, UsageAvailability::Known(_))
                && matches!(row.cache_read_input_tokens, UsageAvailability::Known(_))
        }),
        "unexpected controlled canary usage rows: {rows:#?}"
    );
    print_canary_evidence(&rows, request_bytes);
}

