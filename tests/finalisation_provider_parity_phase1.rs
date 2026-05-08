use std::fs;
use std::path::Path;

use archon_llm::agentic::{AgenticTurnRequest, ProviderCapabilitySet};
use archon_llm::providers::ProviderCapability;

#[test]
fn phase1_agentic_contract_is_exported_from_archon_llm() {
    let request = AgenticTurnRequest {
        model: "model-a".into(),
        max_tokens: 64,
        system: Vec::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_results: Vec::new(),
        effort: None,
        reasoning_encrypted: None,
        session_id: Some("session-a".into()),
        run_id: Some("run-a".into()),
        parent_id: None,
        budget_usd_remaining: Some(1.25),
        extra: serde_json::Value::Null,
    };

    assert_eq!(request.model, "model-a");
    assert_eq!(request.session_id.as_deref(), Some("session-a"));
    assert_eq!(request.run_id.as_deref(), Some("run-a"));
    assert_eq!(request.budget_usd_remaining, Some(1.25));
}

#[test]
fn phase1_chat_streaming_routes_through_agentic_contract() {
    let chat = read("src/command/chat.rs");

    assert!(chat.contains("LlmProviderAgenticAdapter"));
    assert!(chat.contains("AgenticTurnEvent::TextDelta"));
    assert!(chat.contains("stream_turn(request.into(), sink)"));
}

#[test]
fn phase1_agentic_module_does_not_construct_anthropic_directly() {
    for path in [
        "crates/archon-llm/src/agentic/mod.rs",
        "crates/archon-llm/src/agentic/adapter.rs",
        "crates/archon-llm/src/agentic/tests.rs",
    ] {
        let text = read(path);
        assert!(
            !text.contains("AnthropicClient::new"),
            "{path} must stay provider-neutral"
        );
    }
}

#[test]
fn phase1_provider_capability_set_stays_precise() {
    let set =
        ProviderCapabilitySet::new([ProviderCapability::Streaming, ProviderCapability::ToolUse]);

    assert!(set.supports(ProviderCapability::Streaming));
    assert!(set.supports(ProviderCapability::ToolUse));
    assert!(!set.supports(ProviderCapability::Subagents));
    assert!(!set.supports(ProviderCapability::PipelineGametheory));
}

#[test]
fn phase5_btw_routes_through_active_session_provider() {
    let session = read("src/session.rs");

    assert!(session.contains("let btw_provider = Arc::clone(&provider);"));
    assert!(session.contains("provider.stream(request).await"));
    assert!(session.contains("request_origin: Some(\"btw\".into())"));
    assert!(!session.contains("btw side questions are not available in Codex-backed sessions yet"));
}

#[test]
fn phase4_pipelines_use_provider_neutral_adapter() {
    // Post-refactor (commit b8dc02a "Implement audited pipeline runtime"):
    // pipeline.rs was split; the provider-neutral adapter helpers now live in
    // src/command/pipeline_support.rs. The negative asserts still apply to
    // pipeline.rs (Anthropic-specific adapters must not reappear there).
    let pipeline = read("src/command/pipeline.rs");
    let pipeline_support = read("src/command/pipeline_support.rs");
    let gametheory = read("src/command/gametheory.rs");
    let session = read("src/session.rs");

    assert!(
        pipeline_support.contains("build_configured_llm_provider(config, env_vars, origin).await")
    );
    assert!(pipeline_support.contains("ProviderLlmAdapter::new(provider).with_origin(origin)"));
    assert!(!pipeline.contains("AnthropicLlmAdapter::new"));
    assert!(!pipeline.contains("AnthropicClient::new"));

    assert!(gametheory.contains("ProviderLlmAdapter::new(provider).with_origin(\"gametheory\")"));
    assert!(!gametheory.contains("AnthropicLlmAdapter"));
    assert!(!gametheory.contains("AnthropicClient::new"));

    assert!(session.contains("ProviderLlmAdapter::new(Arc::clone(&provider))"));
    assert!(!session.contains("tui-pipeline-device"));
}

#[test]
fn phase3_subagents_and_team_use_active_provider() {
    let team = read("src/command/team.rs");
    let orchestrator = read("crates/archon-core/src/orchestrator.rs");
    let agent = read_all(&[
        "crates/archon-core/src/agent.rs",
        "crates/archon-core/src/agent/lifecycle.rs",
    ]);
    let executor = read_all(&[
        "crates/archon-core/src/subagent_executor.rs",
        "crates/archon-core/src/subagent_executor/run.rs",
    ]);
    let runner = read_all(&[
        "crates/archon-core/src/subagent.rs",
        "crates/archon-core/src/subagent/runner.rs",
        "crates/archon-core/src/subagent/runner/runtime.rs",
    ]);

    assert!(team.contains("build_configured_llm_provider(config, env_vars, \"team\")"));
    assert!(!team.contains("AnthropicClient::new"));
    assert!(!team.contains("resolve_auth_with_keys"));

    assert!(orchestrator.contains("provider: Arc<dyn LlmProvider>"));
    assert!(orchestrator.contains("Agent::new("));
    assert!(orchestrator.contains("self.provider.clone()"));

    assert!(agent.contains("AgentSubagentExecutor::new("));
    assert!(agent.contains("Arc::clone(&self.client)"));
    assert!(executor.contains("client: Arc<dyn LlmProvider>"));
    assert!(executor.contains("SubagentRunner::new("));
    assert!(executor.contains("self.client.clone()"));
    assert!(runner.contains("provider: Arc<dyn LlmProvider>"));
    assert!(runner.contains(".provider\n                .stream(request)"));
    assert!(runner.contains("request_origin: Some(\"subagent\".into())"));
}

#[test]
fn phase5_completion_integrity_is_provider_neutral_today() {
    let completion = read("src/command/completion.rs");
    let completion_lib = read("crates/archon-completion/src/lib.rs");

    assert!(!completion.contains("AnthropicClient::new"));
    assert!(!completion.contains("build_llm_provider"));
    assert!(completion.contains("check_completion_with_context"));
    assert!(completion_lib.contains("trust::recompute_trust_scores_for_run"));
}

#[test]
fn activity_events_and_tui_rows_carry_provider_metadata() {
    let agent = read_all(&[
        "crates/archon-core/src/agent.rs",
        "crates/archon-core/src/agent/events.rs",
        "crates/archon-core/src/agent/tool_context.rs",
    ]);
    let events = read("crates/archon-tui/src/events.rs");
    let rail = read("crates/archon-tui/src/agent_activity.rs");

    assert!(agent.contains("with_provider_model(self.provider.clone(), self.model.clone())"));
    assert!(agent.contains("struct ProviderModelActivitySink"));
    assert!(agent.contains("activity_sink: self.provider_model_activity_sink(active_model)"));
    assert!(events.contains("pub provider: Option<String>"));
    assert!(events.contains("provider: event.provider"));
    assert!(rail.contains("provider={provider}"));
    assert!(rail.contains("model={model}"));
    assert!(rail.contains("cost=${cost:.4}"));
}

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|err| panic!("failed to read {path}: {err}"))
}

fn read_all(paths: &[&str]) -> String {
    paths
        .iter()
        .map(|path| read(path))
        .collect::<Vec<_>>()
        .join("\n")
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
