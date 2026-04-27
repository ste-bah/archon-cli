//! Regression test: verify structural `LlmRequest` fields align between parent
//! and subagent construction (v0.1.18 fix).
//!
//! Subagent dispatches were 429 because `thinking=None`, `speed=None`,
//! `effort=None`, and `max_tokens=16384` diverged from the parent's working
//! request shape. The `AgentConfig::build_base_request_fields` helper is the
//! single source of truth for structural field computation, called by both
//! `agent.rs` and `subagent.rs`. This test locks the helper's output so no
//! refactor can silently re-introduce divergence.

use archon_core::agent::AgentConfig;

#[test]
fn sonnet_gets_adaptive_thinking() {
    let config = AgentConfig::default();
    let (_max_tokens, thinking, _speed) =
        config.build_base_request_fields("claude-sonnet-4-6");
    assert!(thinking.is_some(), "sonnet must have a thinking param");
    let t = thinking.unwrap();
    assert_eq!(t["type"], "adaptive", "sonnet uses adaptive thinking");
}

#[test]
fn opus_gets_adaptive_thinking() {
    let config = AgentConfig::default();
    let (_max_tokens, thinking, _speed) =
        config.build_base_request_fields("claude-opus-4-6");
    assert!(thinking.is_some());
    assert_eq!(thinking.unwrap()["type"], "adaptive");
}

#[test]
fn haiku_gets_budgeted_thinking() {
    let config = AgentConfig::default();
    let (_max_tokens, thinking, _speed) =
        config.build_base_request_fields("claude-haiku-4-5-20251001");
    assert!(thinking.is_some(), "haiku must have thinking when budget > 0");
    let t = thinking.unwrap();
    assert_eq!(t["type"], "enabled", "haiku uses budgeted thinking");
    assert!(t["budget_tokens"].as_u64().unwrap() > 0);
}

#[test]
fn thinking_disabled_for_zero_budget_non_adaptive_model() {
    let mut config = AgentConfig::default();
    config.thinking_budget = 0;
    let (_max_tokens, thinking, _speed) =
        config.build_base_request_fields("gpt-4o");
    assert!(thinking.is_none(), "zero budget disables thinking for non-adaptive models");
}

#[test]
fn max_tokens_uses_config_value() {
    let config = AgentConfig::default();
    let (max_tokens, _thinking, _speed) =
        config.build_base_request_fields("claude-sonnet-4-6");
    assert_eq!(max_tokens, 8192);

    let mut config = AgentConfig::default();
    config.max_tokens = 4096;
    let (max_tokens, _, _) = config.build_base_request_fields("claude-sonnet-4-6");
    assert_eq!(max_tokens, 4096);
}

#[test]
fn speed_defaults_to_none() {
    let config = AgentConfig::default();
    let (_max_tokens, _thinking, speed) =
        config.build_base_request_fields("claude-sonnet-4-6");
    assert_eq!(speed, None, "speed is None when fast_mode is off");
}

#[test]
fn speed_is_fast_when_fast_mode_enabled() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let mut config = AgentConfig::default();
    config.fast_mode = Arc::new(AtomicBool::new(true));
    let (_max_tokens, _thinking, speed) =
        config.build_base_request_fields("claude-sonnet-4-6");
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
        assert!(thinking.is_some(), "thinking must be Some for {model} — this was the 429 bug");
    }
}

#[test]
fn comprehensive_structural_alignment() {
    // The same AgentConfig produces the same (max_tokens, thinking, speed)
    // regardless of which model is used. Both parent and subagent call this
    // same helper, so if this test passes, the alignment is locked.
    let mut config = AgentConfig::default();

    // Test with fast_mode on
    config.fast_mode = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));

    for model in &["claude-sonnet-4-6", "claude-opus-4-6"] {
        let (max_tokens, thinking, speed) = config.build_base_request_fields(model);
        assert_eq!(max_tokens, 8192);
        assert!(thinking.is_some());
        assert_eq!(speed, Some("fast".to_string()));
    }
}
