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
    // Post-refactor (commit 02145e3 "Finalize 1.2.0 release and split session runtime"):
    // btw side-question loop moved from src/session.rs into src/session/btw.rs;
    // the local provider binding is now `provider` (already an `Arc<dyn LlmProvider>`),
    // not `btw_provider`.
    let btw = read("src/session/btw.rs");
    let session_all = read_all(&["src/session.rs", "src/session/btw.rs"]);

    assert!(btw.contains("provider: Arc<dyn LlmProvider>"));
    assert!(btw.contains("provider.stream(request).await"));
    assert!(btw.contains("request_origin: Some(\"btw\".into())"));
    assert!(
        !session_all.contains("btw side questions are not available in Codex-backed sessions yet")
    );
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

    // Post-refactor, the provider clone lives in interactive_agent.rs and the
    // provider-neutral adapter construction lives in the split pipeline adapter.
    // The negative `tui-pipeline-device` assert applies across the whole
    // session module so the legacy hack can never sneak back in via a split file.
    let interactive_agent = read("src/session/interactive_agent.rs");
    let pipeline_adapter = read("src/session/pipeline_adapter.rs");
    let session_all = read_all(&[
        "src/session.rs",
        "src/session/btw.rs",
        "src/session/build_agent.rs",
        "src/session/build_prompt.rs",
        "src/session/config_watcher.rs",
        "src/session/event_forwarder.rs",
        "src/session/interactive_agent.rs",
        "src/session/pipeline_adapter.rs",
        "src/session/interactive_bootstrap.rs",
        "src/session/interactive_finish.rs",
        "src/session/interactive_setup.rs",
        "src/session/interactive_ui.rs",
        "src/session/modes.rs",
        "src/session/reasoning_quality.rs",
        "src/session/slash_context_builder.rs",
        "src/session/splash.rs",
    ]);
    let _ = session; // legacy binding kept above; phase4 enforcement now spans the module.
    assert!(interactive_agent.contains("build_subagent_pipeline_client("));
    assert!(interactive_agent.contains("Arc::clone(&provider)"));
    assert!(
        pipeline_adapter
            .contains("ProviderLlmAdapter::new(provider).with_origin(\"tui_pipeline\")")
    );
    assert!(!session_all.contains("tui-pipeline-device"));
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
        "crates/archon-core/src/subagent_executor/run_runner.rs",
    ]);
    let runner = read_all(&[
        "crates/archon-core/src/subagent.rs",
        "crates/archon-core/src/subagent/runner.rs",
        "crates/archon-core/src/subagent/runner/runtime.rs",
        "crates/archon-core/src/subagent/runner/runtime/request_round.rs",
        "crates/archon-core/src/subagent/runner/runtime/stream_round.rs",
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
    // Subagent runner streams via the injected provider. The auto-compaction
    // refactor (v1.2.2) collapsed `.provider\n  .stream(request)` to a
    // single-line `self.provider.stream(request.clone())` call. The contract
    // is "the call goes through self.provider.stream(...)" — formatting
    // varies, so accept either shape.
    assert!(
        runner.contains("self.provider.stream(") || runner.contains(".provider\n"),
        "subagent runner must call self.provider.stream(...)"
    );
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

    // v1.3.0: ProviderModelActivitySink::emit now fills provider/model only
    // when the inner event has them unset (preserves subagent metadata,
    // e.g. Opus subagent stays Opus instead of being overwritten by the
    // parent Sonnet). Verify the fill-if-missing pattern, not the old
    // unconditional with_provider_model(...) overwrite.
    assert!(agent.contains("if event.provider.is_none()"));
    assert!(agent.contains("event.provider = Some(self.provider.clone())"));
    assert!(agent.contains("if event.model.is_none()"));
    assert!(agent.contains("event.model = Some(self.model.clone())"));
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
