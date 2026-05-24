use super::*;

#[tokio::test]
async fn text_only_returns_immediately() {
    let provider = Arc::new(MockProvider::new(vec![text_response(
        "Hello from subagent",
    )]));
    let runner = make_runner(provider.clone(), 10);
    let result = runner.run("Say hello").await.unwrap();
    assert_eq!(result, "Hello from subagent");
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 1);
}

struct AliasRecordingProvider {
    seen_models: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl LlmProvider for AliasRecordingProvider {
    fn name(&self) -> &str {
        "alias-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "gpt-resolved".into(),
            display_name: "GPT Resolved".into(),
            context_window: 400_000,
        }]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.seen_models.lock().unwrap().push(request.model);
        let events = text_response("resolved");
        let (tx, rx) = mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn tier_alias_is_resolved_before_subagent_request() {
    let provider = Arc::new(AliasRecordingProvider {
        seen_models: std::sync::Mutex::new(Vec::new()),
    });
    let runner = make_runner_with_model(provider.clone(), 10, AgentConfig::default(), "sonnet");

    let result = runner.run("Say hello").await.unwrap();

    assert_eq!(result, "resolved");
    assert_eq!(runner.model(), "gpt-resolved");
    assert_eq!(
        provider.seen_models.lock().unwrap().as_slice(),
        ["gpt-resolved"]
    );
}

#[tokio::test]
async fn tool_use_then_text_returns_after_two_turns() {
    let provider = Arc::new(MockProvider::new(vec![
        // Turn 1: LLM uses a tool
        tool_use_response("tool-1", "Read", r#"{"file_path":"/tmp/test.txt"}"#),
        // Turn 2: LLM returns text
        text_response("I read the file."),
    ]));
    let runner = make_runner(provider.clone(), 10);
    let result = runner.run("Read /tmp/test.txt").await.unwrap();
    assert_eq!(result, "I read the file.");
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn signed_thinking_is_replayed_for_anthropic_shape_after_tool_use() {
    let provider = Arc::new(MockProvider::new_with_family(
        vec![
            thinking_tool_use_response("tool-1", "Read", r#"{"file_path":"/tmp/test.txt"}"#),
            text_response("done"),
        ],
        ProviderFamily::AnthropicApi,
    ));
    let runner = make_runner(provider.clone(), 10);

    let result = runner.run("Read /tmp/test.txt").await.unwrap();

    assert_eq!(result, "done");
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);

    let assistant_msg = requests[1]
        .messages
        .iter()
        .find(|msg| msg.get("role").and_then(|role| role.as_str()) == Some("assistant"))
        .expect("assistant tool-use turn is replayed");
    let content = assistant_msg
        .get("content")
        .and_then(|value| value.as_array())
        .expect("assistant content is block array");

    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["thinking"], "checking the file before answering");
    assert_eq!(content[0]["signature"], "signed-thinking");
    assert_eq!(content[1]["type"], "tool_use");
}

#[tokio::test]
async fn encrypted_reasoning_is_replayed_without_raw_thinking_for_codex_shape() {
    let provider = Arc::new(MockProvider::new_with_family(
        vec![
            thinking_tool_use_response("tool-1", "Read", r#"{"file_path":"/tmp/test.txt"}"#),
            text_response("done"),
        ],
        ProviderFamily::CodexOAuth,
    ));
    let runner = make_runner(provider.clone(), 10);

    let result = runner.run("Read /tmp/test.txt").await.unwrap();

    assert_eq!(result, "done");
    let requests = provider.requests();
    assert_eq!(
        requests[1].reasoning_encrypted.as_deref(),
        Some("opaque-codex-reasoning")
    );

    let assistant_msg = requests[1]
        .messages
        .iter()
        .find(|msg| msg.get("role").and_then(|role| role.as_str()) == Some("assistant"))
        .expect("assistant tool-use turn is replayed");
    let content = assistant_msg
        .get("content")
        .and_then(|value| value.as_array())
        .expect("assistant content is block array");

    assert!(
        content
            .iter()
            .all(|block| block.get("type").and_then(|value| value.as_str()) != Some("thinking")),
        "Codex/OpenAI Responses continuation should use encrypted reasoning, not raw thinking"
    );
    assert_eq!(content[0]["type"], "tool_use");
}

#[tokio::test]
async fn max_turns_enforced() {
    // Every turn uses a tool, never returns text
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("t1", "Read", r#"{"file_path":"/tmp/a"}"#),
        tool_use_response("t2", "Read", r#"{"file_path":"/tmp/b"}"#),
        tool_use_response("t3", "Read", r#"{"file_path":"/tmp/c"}"#),
    ]));
    let runner = make_runner(provider.clone(), 2);
    let err = runner.run("keep going").await.unwrap_err();
    assert!(err.to_string().contains("max turns (2)"));
    assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn api_error_propagated() {
    let provider = Arc::new(MockProvider::new(vec![vec![StreamEvent::Error {
        error_type: "server_error".into(),
        message: "internal failure".into(),
    }]]));
    let runner = make_runner(provider, 10);
    let err = runner.run("trigger error").await.unwrap_err();
    assert!(err.to_string().contains("internal failure"));
}

#[derive(Default)]
struct CompactionFailsProvider {
    origins: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl LlmProvider for CompactionFailsProvider {
    fn name(&self) -> &str {
        "compaction-fails"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, archon_llm::provider::LlmError> {
        let origin = request.request_origin.unwrap_or_else(|| "unknown".into());
        self.origins.lock().unwrap().push(origin.clone());
        if origin == "compaction_summary" {
            return Err(archon_llm::provider::LlmError::RateLimited {
                retry_after_secs: 30,
            });
        }
        let events = text_response("survived");
        let (tx, rx) = mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }

    async fn complete(
        &self,
        _request: LlmRequest,
    ) -> Result<LlmResponse, archon_llm::provider::LlmError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn proactive_compaction_failure_falls_through_same_turn() {
    let provider = Arc::new(CompactionFailsProvider::default());
    let mut config = AgentConfig::default();
    config.context.context_window_override = Some(100);
    config.context.output_reserve_tokens = 0;
    config.context.compact_threshold = 0.01;
    config.context.preflight_safety_margin = 0.0;
    let runner = make_runner_with_config(provider.clone(), 5, config);

    let result = runner.run("trigger compact, then answer").await.unwrap();

    assert_eq!(result, "survived");
    let origins = provider.origins.lock().unwrap().clone();
    assert_eq!(origins, vec!["compaction_summary", "subagent"]);
}

#[tokio::test]
async fn isolated_messages() {
    // Verify that each run starts with fresh messages
    let provider = Arc::new(MockProvider::new(vec![text_response("First run")]));
    let runner = make_runner(provider.clone(), 10);
    let r1 = runner.run("First prompt").await.unwrap();
    assert_eq!(r1, "First run");

    // Second run should start fresh (provider returns default "(done)")
    let r2 = runner.run("Second prompt").await.unwrap();
    assert_eq!(r2, "(done)");
}

#[tokio::test]
async fn tool_dispatch_error_continues() {
    // Use a nonexistent tool — dispatch should return error, loop continues
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("t1", "NonexistentTool", r#"{}"#),
        text_response("Recovered after tool error"),
    ]));
    let runner = make_runner(provider.clone(), 10);
    let result = runner.run("use bad tool").await.unwrap();
    assert_eq!(result, "Recovered after tool error");
}

#[tokio::test]
async fn empty_tool_definitions_still_works() {
    let provider = Arc::new(MockProvider::new(vec![text_response("No tools needed")]));
    let registry = Arc::new(crate::dispatch::create_default_registry(
        std::env::current_dir().unwrap_or_default(),
        None,
    ));
    let ctx = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: "test".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    };
    let runner = SubagentRunner::new(
        provider,
        "Test agent".into(),
        vec![], // Empty tool defs
        registry,
        ctx,
        "mock".into(),
        5,
        60,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    );
    let result = runner.run("hello").await.unwrap();
    assert_eq!(result, "No tools needed");
}

// snip_context_preserves_recent_turns: removed. archon no longer performs
// client-side context truncation in the subagent runner — context flows to
// the configured LLM provider verbatim, which is the source of truth for
// context limits.
