//! Regression test: verify structural `LlmRequest` fields align between parent
//! and subagent construction (v0.1.18 fix).
//!
//! Subagent dispatches were 429 because `thinking=None`, `speed=None`,
//! `effort=None`, and `max_tokens=16384` diverged from the parent's working
//! request shape. The `AgentConfig::build_base_request_fields` helper is the
//! single source of truth for structural field computation, called by both
//! `agent.rs` and `subagent.rs`. This test locks the helper's output so no
//! refactor can silently re-introduce divergence.
//!
//! v0.1.19 adds billing-header alignment: the subagent's system prompt
//! must prepend the same billing-header text block the parent uses,
//! but only when IdentityMode::Spoof is active.

use std::sync::Arc;

use archon_core::agent::AgentConfig;
use archon_core::dispatch::ToolRegistry;
use archon_core::subagent::runner::SubagentRunner;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_tools::tool::ToolContext;
use tokio::sync::Mutex;

#[test]
fn sonnet_gets_adaptive_thinking() {
    let config = AgentConfig::default();
    let (_max_tokens, thinking, _speed) = config.build_base_request_fields("claude-sonnet-4-6");
    assert!(thinking.is_some(), "sonnet must have a thinking param");
    let t = thinking.unwrap();
    assert_eq!(t["type"], "adaptive", "sonnet uses adaptive thinking");
}

#[test]
fn opus_gets_adaptive_thinking() {
    let config = AgentConfig::default();
    let (_max_tokens, thinking, _speed) = config.build_base_request_fields("claude-opus-4-6");
    assert!(thinking.is_some());
    assert_eq!(thinking.unwrap()["type"], "adaptive");
}

#[test]
fn haiku_gets_budgeted_thinking() {
    let config = AgentConfig::default();
    let (_max_tokens, thinking, _speed) =
        config.build_base_request_fields("claude-haiku-4-5-20251001");
    assert!(
        thinking.is_some(),
        "haiku must have thinking when budget > 0"
    );
    let t = thinking.unwrap();
    assert_eq!(t["type"], "enabled", "haiku uses budgeted thinking");
    assert!(t["budget_tokens"].as_u64().unwrap() > 0);
}

#[test]
fn thinking_disabled_for_zero_budget_non_adaptive_model() {
    let config = AgentConfig {
        thinking_budget: 0,
        ..AgentConfig::default()
    };
    let (_max_tokens, thinking, _speed) = config.build_base_request_fields("gpt-4o");
    assert!(
        thinking.is_none(),
        "zero budget disables thinking for non-adaptive models"
    );
}

#[test]
fn max_tokens_uses_config_value() {
    let config = AgentConfig::default();
    let (max_tokens, _thinking, _speed) = config.build_base_request_fields("claude-sonnet-4-6");
    assert_eq!(max_tokens, 8192);

    let config = AgentConfig {
        max_tokens: 4096,
        ..AgentConfig::default()
    };
    let (max_tokens, _, _) = config.build_base_request_fields("claude-sonnet-4-6");
    assert_eq!(max_tokens, 4096);
}

#[test]
fn speed_defaults_to_none() {
    let config = AgentConfig::default();
    let (_max_tokens, _thinking, speed) = config.build_base_request_fields("claude-sonnet-4-6");
    assert_eq!(speed, None, "speed is None when fast_mode is off");
}

#[test]
fn speed_is_fast_when_fast_mode_enabled() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let config = AgentConfig {
        fast_mode: Arc::new(AtomicBool::new(true)),
        ..AgentConfig::default()
    };
    let (_max_tokens, _thinking, speed) = config.build_base_request_fields("claude-sonnet-4-6");
    assert_eq!(speed, Some("fast".to_string()));
}

#[test]
fn alignment_regression_all_models() {
    // Every model that both parent and subagent can use must produce valid,
    // non-None thinking params. This is the key v0.1.18 invariant: thinking
    // was None for subagents on all models, triggering 429.
    let config = AgentConfig::default();

    let models = &[
        "claude-sonnet-4-6",
        "claude-opus-4-6",
        "claude-haiku-4-5-20251001",
    ];

    for model in models {
        let (max_tokens, thinking, _speed) = config.build_base_request_fields(model);
        assert!(max_tokens > 0, "max_tokens must be positive for {model}");
        assert!(
            thinking.is_some(),
            "thinking must be Some for {model} — this was the 429 bug"
        );
    }
}

#[test]
fn comprehensive_structural_alignment() {
    // The same AgentConfig produces the same (max_tokens, thinking, speed)
    // regardless of which model is used. Both parent and subagent call this
    // same helper, so if this test passes, the alignment is locked.
    let config = AgentConfig {
        fast_mode: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        ..AgentConfig::default()
    };

    for model in &["claude-sonnet-4-6", "claude-opus-4-6"] {
        let (max_tokens, thinking, speed) = config.build_base_request_fields(model);
        assert_eq!(max_tokens, 8192);
        assert!(thinking.is_some());
        assert_eq!(speed, Some("fast".to_string()));
    }
}

// ---------------------------------------------------------------------------
// v0.1.19: billing-header system block alignment tests
// ---------------------------------------------------------------------------

struct CapturingMockProvider {
    captured_request: Arc<Mutex<Option<LlmRequest>>>,
}

impl CapturingMockProvider {
    fn new(captured_request: Arc<Mutex<Option<LlmRequest>>>) -> Self {
        Self { captured_request }
    }
}

#[async_trait::async_trait]
impl LlmProvider for CapturingMockProvider {
    fn name(&self) -> &str {
        "capturing-mock"
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
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        // Capture the request before returning the response
        *self.captured_request.lock().await = Some(request);

        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let events = vec![
            StreamEvent::MessageStart {
                id: "msg-1".into(),
                model: "mock".into(),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
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
                text: "ok".into(),
            },
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::MessageStop,
        ];
        for event in events {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

fn make_runner(provider: Arc<CapturingMockProvider>, identity: IdentityProvider) -> SubagentRunner {
    let tool_registry = ToolRegistry::new();
    let tool_defs = vec![];
    let ctx = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: "test".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        in_fork: false,
        nested: false,
        cancel_parent: None,
        sandbox: None,
    };
    SubagentRunner::new(
        provider,
        "Test agent body".into(),
        tool_defs,
        Arc::new(tool_registry),
        ctx,
        "mock-model".into(),
        1,
        60,
        Arc::new(AgentConfig::default()),
        Arc::new(identity),
    )
}

#[tokio::test]
async fn subagent_system_block_starts_with_billing_header_in_spoof_mode() {
    let captured = Arc::new(Mutex::new(None));
    let provider = Arc::new(CapturingMockProvider::new(Arc::clone(&captured)));

    let identity = IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".into(),
            entrypoint: "cli".into(),
            betas: vec![],
            workload: None,
            anti_distillation: false,
        },
        "test-session".into(),
        "test-device".into(),
        String::new(),
    );

    let runner = make_runner(provider, identity);
    let _ = runner.run("hello").await;

    let request = captured
        .lock()
        .await
        .take()
        .expect("request should be captured");

    // system[0] must be the billing-header block
    let block0 = &request.system[0];
    assert_eq!(block0["type"], "text");
    let text0 = block0["text"].as_str().unwrap();
    assert!(
        text0.starts_with("x-anthropic-billing-header:"),
        "system[0] text must start with billing header, got: {text0}"
    );
    assert!(text0.contains("cc_version=2.1.89"));
    assert!(text0.contains("cc_entrypoint=cli;"));

    // system must have at least 2 blocks (billing + body)
    assert!(
        request.system.len() >= 2,
        "expected at least 2 system blocks, got {}",
        request.system.len()
    );

    // One of the subsequent blocks must contain the agent body
    let body_found = request.system[1..].iter().any(|b| {
        b["text"]
            .as_str()
            .is_some_and(|t| t.contains("Test agent body"))
    });
    assert!(body_found, "agent body not found in system blocks");
}

#[tokio::test]
async fn subagent_system_block_omits_billing_header_in_clean_mode() {
    let captured = Arc::new(Mutex::new(None));
    let provider = Arc::new(CapturingMockProvider::new(Arc::clone(&captured)));

    let identity = IdentityProvider::new(
        IdentityMode::Clean,
        "test-session".into(),
        "test-device".into(),
        String::new(),
    );

    let runner = make_runner(provider, identity);
    let _ = runner.run("hello").await;

    let request = captured
        .lock()
        .await
        .take()
        .expect("request should be captured");

    // No system block should start with billing header
    for block in &request.system {
        if let Some(text) = block["text"].as_str() {
            assert!(
                !text.starts_with("x-anthropic-billing-header:"),
                "Clean mode must not prepend billing header, but found: {text}"
            );
        }
    }

    // Body should be present
    let body_found = request.system.iter().any(|b| {
        b["text"]
            .as_str()
            .is_some_and(|t| t.contains("Test agent body"))
    });
    assert!(body_found, "agent body not found in system blocks");
}
