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

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|err| panic!("failed to read {path}: {err}"))
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
