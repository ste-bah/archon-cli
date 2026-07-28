use super::*;
use crate::runner::{AgentInfo, PipelineType, ToolAccessLevel};
use archon_llm::provider::{LlmError, ModelInfo, ProviderFeature};
use archon_llm::types::Usage;

struct FakeProvider {
    name: &'static str,
    model: &'static str,
    context_window: u32,
    seen_model: std::sync::Mutex<Option<String>>,
    seen_messages: std::sync::Mutex<Vec<serde_json::Value>>,
    seen_extra: std::sync::Mutex<Option<serde_json::Value>>,
}

#[async_trait]
impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: self.model.into(),
            display_name: self.model.into(),
            context_window: self.context_window,
        }]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        *self
            .seen_model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.model);
        *self
            .seen_extra
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.extra.clone());
        *self
            .seen_messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = request.messages;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let _ = tx
                .send(StreamEvent::MessageStart {
                    id: "msg_fake".into(),
                    model: "gpt-5.4".into(),
                    usage: Usage {
                        input_tokens: 7,
                        ..Default::default()
                    },
                })
                .await;
            let _ = tx
                .send(StreamEvent::TextDelta {
                    index: 0,
                    text: "pipeline-ok".into(),
                })
                .await;
            let _ = tx.send(StreamEvent::MessageStop).await;
        });
        Ok(rx)
    }

    async fn complete(
        &self,
        _request: LlmRequest,
    ) -> Result<archon_llm::provider::LlmResponse, LlmError> {
        Err(LlmError::Unsupported("fake provider complete".into()))
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(feature, ProviderFeature::Streaming)
    }
}

#[tokio::test]
async fn provider_adapter_rounds_are_distinct_within_one_scope() {
    let provider = Arc::new(FakeProvider {
        name: "openai-codex",
        model: "gpt-5.4",
        context_window: 123_456,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let adapter = ProviderLlmAdapter::new(provider).with_origin("workflow");

    let first = adapter.runtime_extra("run-1", "session-1");
    let second = adapter.runtime_extra("run-1", "session-1");

    assert_eq!(first["archon_runtime"]["round"], 0);
    assert_eq!(second["archon_runtime"]["round"], 1);
    assert_eq!(
        first["archon_runtime"]["session_id"],
        second["archon_runtime"]["session_id"]
    );
}

#[tokio::test]
async fn provider_adapter_uses_agent_execution_session_scope() {
    let provider = Arc::new(FakeProvider {
        name: "openai-codex",
        model: "gpt-5.4",
        context_window: 123_456,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let seen = Arc::clone(&provider);
    let adapter = ProviderLlmAdapter::new(provider).with_origin("workflow");
    let request = AgentExecutionRequest {
        session_id: "pipeline-session-42".into(),
        pipeline_type: PipelineType::Workflow,
        task: "run workflow".into(),
        cwd: None,
        ordinal: 0,
        attempt: 1,
        agent: AgentInfo {
            key: "reviewer".into(),
            display_name: "Reviewer".into(),
            model: "gpt-5.4".into(),
            phase: 1,
            critical: false,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access_level: ToolAccessLevel::ReadOnly,
        },
        messages: Vec::new(),
        system: Vec::new(),
        tools: Vec::new(),
        allowed_tools: Vec::new(),
        timeout_secs: None,
        disable_auto_background: false,
        provider_env_resolution: None,
    };

    adapter
        .run_agent(request)
        .await
        .expect("fake provider response");

    let extra = seen
        .seen_extra
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .expect("request attribution");
    assert_eq!(extra["archon_runtime"]["run_id"], "pipeline-session-42");
    assert_eq!(extra["archon_runtime"]["session_id"], "pipeline-session-42");
    assert_eq!(extra["archon_runtime"]["round"], 0);
}

#[tokio::test]
async fn provider_adapter_attaches_pipeline_attribution() {
    let provider = Arc::new(FakeProvider {
        name: "openai-codex",
        model: "gpt-5.4",
        context_window: 123_456,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let seen = Arc::clone(&provider);
    let adapter = ProviderLlmAdapter::new(provider).with_origin("workflow");

    adapter
        .send_message(Vec::new(), Vec::new(), Vec::new(), "gpt-5.4")
        .await
        .expect("fake provider response");

    let extra = seen
        .seen_extra
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .expect("request attribution");
    assert_eq!(extra["archon_runtime"]["role"], "pipeline");
    assert_eq!(extra["archon_runtime"]["origin"], "workflow");
    assert!(extra["archon_runtime"]["run_id"].as_str().is_some());
    assert!(extra["archon_runtime"]["session_id"].as_str().is_some());
    assert_eq!(extra["archon_runtime"]["round"], 0);
    assert!(extra["archon_runtime"].get("turn").is_none());
    assert!(
        extra["archon_runtime"]
            .get("effective_denominator")
            .is_none()
    );
}

#[tokio::test]
async fn provider_adapter_collects_text_from_generic_provider() {
    let provider = Arc::new(FakeProvider {
        name: "openai-codex",
        model: "gpt-5.4",
        context_window: 123_456,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let adapter = ProviderLlmAdapter::new(provider);

    let response = adapter
        .send_message(Vec::new(), Vec::new(), Vec::new(), "gpt-5.4")
        .await
        .expect("fake provider response");

    assert_eq!(response.content, "pipeline-ok");
    assert_eq!(response.tokens_in, 7);
}

#[tokio::test]
async fn provider_adapter_rejects_cwd_bound_agent_requests() {
    let provider = Arc::new(FakeProvider {
        name: "openai-codex",
        model: "gpt-5.4",
        context_window: 123_456,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let adapter = ProviderLlmAdapter::new(provider);

    let err = adapter
        .run_agent(AgentExecutionRequest {
            session_id: "s".into(),
            pipeline_type: PipelineType::Workflow,
            task: "edit repo".into(),
            cwd: Some("/target/repo".into()),
            ordinal: 1,
            attempt: 1,
            agent: AgentInfo {
                key: "coder".into(),
                display_name: "Coder".into(),
                model: "sonnet".into(),
                phase: 0,
                critical: false,
                parallelizable: false,
                quality_threshold: 0.5,
                tool_access_level: ToolAccessLevel::Full,
            },
            messages: Vec::new(),
            system: Vec::new(),
            tools: Vec::new(),
            allowed_tools: Vec::new(),
            timeout_secs: None,
            disable_auto_background: false,
            provider_env_resolution: None,
        })
        .await
        .expect_err("raw provider adapter must not ignore cwd-bound agents");

    assert!(
        err.to_string()
            .contains("wrap it in SubagentPipelineClient"),
        "{err}"
    );
}

#[tokio::test]
async fn provider_adapter_remaps_claude_agent_model_to_provider_default() {
    let provider = Arc::new(FakeProvider {
        name: "openai-codex",
        model: "gpt-5.4",
        context_window: 123_456,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let seen = Arc::clone(&provider);
    let adapter = ProviderLlmAdapter::new(provider);

    let _ = adapter
        .send_message(Vec::new(), Vec::new(), Vec::new(), "claude-sonnet-4-6")
        .await
        .expect("fake provider response");

    let model = seen
        .seen_model
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(model.as_deref(), Some("gpt-5.4"));
}

#[tokio::test]
async fn provider_adapter_resolves_tier_alias_to_provider_default() {
    let provider = Arc::new(FakeProvider {
        name: "deepseek",
        model: "deepseek-chat",
        context_window: 123_456,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let seen = Arc::clone(&provider);
    let adapter = ProviderLlmAdapter::new(provider);

    let _ = adapter
        .send_message(Vec::new(), Vec::new(), Vec::new(), "sonnet")
        .await
        .expect("fake provider response");

    assert_eq!(adapter.provider_id().as_deref(), Some("deepseek"));
    assert_eq!(adapter.resolve_model_alias("sonnet"), "deepseek-chat");
    let model = seen
        .seen_model
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(model.as_deref(), Some("deepseek-chat"));
}

#[tokio::test]
async fn provider_adapter_keeps_prompt_budgeting_out_of_adapter() {
    let provider = Arc::new(FakeProvider {
        name: "openai-codex",
        model: "gpt-5.4",
        context_window: 128,
        seen_model: std::sync::Mutex::new(None),
        seen_messages: std::sync::Mutex::new(Vec::new()),
        seen_extra: std::sync::Mutex::new(None),
    });
    let seen = Arc::clone(&provider);
    let adapter = ProviderLlmAdapter::new(provider);
    let messages: Vec<_> = (0..10)
        .map(|i| serde_json::json!({"role": "user", "content": "x".repeat(100 + i)}))
        .collect();

    let _ = adapter
        .send_message(messages.clone(), Vec::new(), Vec::new(), "gpt-5.4")
        .await
        .expect("fake provider response");

    let sent = seen
        .seen_messages
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(sent.len(), messages.len());
}
