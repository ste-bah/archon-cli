//! Tests for the `/model` slash-command handler.
//!
//! Split out of `model.rs` to keep that file under the 500-line ceiling. Same
//! `#[path]` include convention the rest of the tree uses, so the module still
//! sees `super::*` and nothing about the tests changed.

use super::*;
use crate::command::test_support::*;

fn snapshot(current_model: &str) -> ModelSnapshot {
    ModelSnapshot {
        current_model: current_model.to_string(),
        codex_models: archon_core::config::OpenAiCodexModelsConfig::default(),
        anthropic_models: archon_core::config::AnthropicModelsConfig::default(),
    }
}

fn anthropic_snapshot(current_model: &str) -> ModelSnapshot {
    snapshot(current_model)
}

fn codex_snapshot(current_model: &str) -> ModelSnapshot {
    snapshot(current_model)
}

fn provider_snapshot(current_model: &str) -> ModelSnapshot {
    snapshot(current_model)
}

#[test]
fn model_handler_description_matches() {
    let h = ModelHandler;
    let desc = h.description();
    assert!(
        !desc.is_empty(),
        "ModelHandler description must be non-empty"
    );
    assert!(
        desc.to_lowercase().contains("model"),
        "ModelHandler description should reference 'model', got: {desc}"
    );
}

#[test]
fn model_handler_aliases_are_m_and_switch_model() {
    let h = ModelHandler;
    assert_eq!(
        h.aliases(),
        &["m", "switch-model"],
        "ModelHandler aliases must be [m, switch-model] per AGS-808 spec"
    );
}

#[test]
fn model_handler_execute_no_args_emits_current_model_text() {
    let snap = anthropic_snapshot("opus");
    let (mut ctx, mut rx) = make_model_ctx(Some(snap));
    let h = ModelHandler;
    h.execute(&mut ctx, &[])
        .expect("ModelHandler::execute must return Ok with snapshot populated");

    let ev = rx.try_recv().expect("must emit a TuiEvent");
    match ev {
        TuiEvent::TextDelta(msg) => {
            assert!(
                msg.contains("Current model: opus"),
                "TextDelta must contain 'Current model: opus', got: {msg}"
            );
            assert!(
                msg.contains("Usage: /model <name>"),
                "TextDelta must contain the usage line, got: {msg}"
            );
            assert!(
                msg.contains("Shortcuts: opus, sonnet, haiku"),
                "TextDelta must contain shortcuts line, got: {msg}"
            );
        }
        other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
    }
    // READ path must NOT stash a CommandEffect (nothing to apply).
    assert!(
        ctx.pending_effect.is_none(),
        "READ path must not produce a CommandEffect"
    );
}

#[test]
fn model_handler_execute_no_args_without_snapshot_returns_err() {
    let (mut ctx, _rx) = make_model_ctx(None);
    let h = ModelHandler;
    let result = h.execute(&mut ctx, &[]);
    assert!(
        result.is_err(),
        "ModelHandler::execute must return Err when model_snapshot is None \
         (defensive: builder bug should surface loudly)"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("model_snapshot") || err_msg.contains("build_command_context"),
        "error must describe the missing snapshot, got: {err_msg}"
    );
}

#[test]
fn model_handler_execute_with_valid_arg_sets_effect_and_emits_events() {
    let (mut ctx, mut rx) = make_model_ctx(Some(anthropic_snapshot("claude-sonnet-4-6")));
    let h = ModelHandler;
    h.execute(&mut ctx, &["opus".to_string()])
        .expect("valid arg must produce Ok(())");

    // `opus` resolves through `[models.anthropic]`, not through
    // `KNOWN_SHORTCUTS`. This asserted the literal `claude-opus-4-8` and broke
    // when the alias began reading config — the literal was a second copy of a
    // default, which is the drift the change removed.
    let expected = archon_core::config::AnthropicModelsConfig::default().opus;
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetModelOverride(s)) => {
            assert_eq!(
                s, &expected,
                "pending_effect must carry the resolved full model id"
            );
        }
        // TASK-AGS-POST-6-BODIES-B04-DIFF: RunGitDiffStat belongs to
        // /diff. The /model WRITE path must never stash it; this
        // arm pins that boundary and keeps the match exhaustive.
        Some(other) => panic!(
            "unexpected CommandEffect variant for /model WRITE path: {:?}",
            other
        ),
        None => panic!("WRITE path must stash a CommandEffect::SetModelOverride"),
    }

    // Collect events in emission order.
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    // Expect at least ModelChanged + TextDelta ("Model switched...").
    let mut saw_model_changed = false;
    let mut saw_text_delta = false;
    for ev in &events {
        match ev {
            TuiEvent::ModelChanged(s) => {
                assert_eq!(s, &expected);
                saw_model_changed = true;
            }
            TuiEvent::TextDelta(msg) if msg.contains(&format!("Model switched to {expected}")) => {
                saw_text_delta = true;
            }
            _ => {}
        }
    }
    assert!(
        saw_model_changed,
        "WRITE path must emit TuiEvent::ModelChanged"
    );
    assert!(
        saw_text_delta,
        "WRITE path must emit TuiEvent::TextDelta with 'Model switched to ...'"
    );
}

/// Codex aliases come from `[models.openai-codex]`, not from a literal.
///
/// The regression this pins: `default`, `opus` and `sonnet` all returned a
/// hardcoded `"gpt-5.5"`, so an operator who set a different frontier model
/// in config got the old one anyway — and silently, because a valid model
/// ID came back either way. Configuring values that share no prefix with
/// the previous constants means a stale implementation cannot coincidentally
/// pass.
#[test]
fn codex_aliases_resolve_from_config_not_a_hardcoded_id() {
    let cfg = archon_core::config::OpenAiCodexModelsConfig {
        default: "gpt-5.6-sol".into(),
        codex: "gpt-5.6-codex".into(),
        mini: "gpt-5.6-nano".into(),
    };

    for alias in ["default", "opus", "sonnet"] {
        assert_eq!(
            resolve_codex_model_name(alias, &cfg).unwrap(),
            "gpt-5.6-sol",
            "'{alias}' must resolve through [models.openai-codex].default"
        );
    }
    assert_eq!(
        resolve_codex_model_name("codex", &cfg).unwrap(),
        "gpt-5.6-codex"
    );
    for alias in ["mini", "haiku"] {
        assert_eq!(
            resolve_codex_model_name(alias, &cfg).unwrap(),
            "gpt-5.6-nano",
            "'{alias}' must resolve through [models.openai-codex].mini"
        );
    }

    // A literal ID still passes through untouched.
    assert_eq!(
        resolve_codex_model_name("gpt-5.4", &cfg).unwrap(),
        "gpt-5.4"
    );
}

/// Anthropic aliases come from `[models.anthropic]`, not from a literal.
///
/// The mirror of `codex_aliases_resolve_from_config_not_a_hardcoded_id`, and
/// the same defect: `opus`/`sonnet`/`haiku` resolved through
/// `archon_tools::validation::KNOWN_SHORTCUTS`, whose own doc comment names
/// `resolve_anthropic_model(alias, &cfg)` as the correct route — a resolver
/// that had no callers. Configured values sharing no prefix with the old
/// constants, so a stale implementation cannot coincidentally pass.
#[test]
fn anthropic_aliases_resolve_from_config_not_a_hardcoded_id() {
    let cfg = archon_core::config::AnthropicModelsConfig {
        opus: "claude-opus-9".into(),
        sonnet: "claude-sonnet-9".into(),
        haiku: "claude-haiku-9".into(),
    };

    assert_eq!(
        resolve_anthropic_model_name("opus", &cfg).unwrap(),
        "claude-opus-9"
    );
    assert_eq!(
        resolve_anthropic_model_name("sonnet", &cfg).unwrap(),
        "claude-sonnet-9"
    );
    assert_eq!(
        resolve_anthropic_model_name("haiku", &cfg).unwrap(),
        "claude-haiku-9"
    );

    // A literal ID still passes through validation untouched.
    assert_eq!(
        resolve_anthropic_model_name("claude-opus-4-8", &cfg).unwrap(),
        "claude-opus-4-8"
    );
    // And a typo still gets the did-you-mean path rather than being accepted.
    assert!(resolve_anthropic_model_name("definitely-not-a-model", &cfg).is_err());
}

/// End-to-end through the handler: `/model opus` must select what config says.
#[test]
fn model_handler_anthropic_alias_uses_configured_model() {
    let (mut ctx, _rx) = make_model_ctx(Some(anthropic_snapshot("claude-sonnet-4-6")));
    let h = ModelHandler;
    h.execute(&mut ctx, &["opus".to_string()])
        .expect("Anthropic alias must produce Ok(())");

    let expected = archon_core::config::AnthropicModelsConfig::default().opus;
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetModelOverride(s)) => assert_eq!(
            s, &expected,
            "/model opus must select [models.anthropic].opus, not a compile-time literal"
        ),
        Some(other) => panic!("unexpected CommandEffect variant: {other:?}"),
        None => panic!("WRITE path must stash a CommandEffect::SetModelOverride"),
    }
}

#[test]
fn model_handler_execute_with_invalid_arg_emits_error_no_effect() {
    let (mut ctx, mut rx) = make_model_ctx(Some(anthropic_snapshot("claude-sonnet-4-6")));
    let h = ModelHandler;
    h.execute(&mut ctx, &["definitely-not-a-model-xyz".to_string()])
        .expect("invalid arg path still returns Ok(()) — error is emitted as event");

    assert!(
        ctx.pending_effect.is_none(),
        "invalid WRITE must NOT stash an effect"
    );

    let ev = rx
        .try_recv()
        .expect("invalid arg must emit a TuiEvent::Error");
    match ev {
        TuiEvent::Error(msg) => {
            assert!(
                !msg.is_empty(),
                "Error message must be non-empty, got empty"
            );
        }
        other => panic!("expected TuiEvent::Error, got {other:?}"),
    }
}

#[test]
fn model_handler_execute_with_codex_literal_sets_effect_and_emits_events() {
    let (mut ctx, mut rx) = make_model_ctx(Some(codex_snapshot("gpt-5.4")));
    let h = ModelHandler;
    h.execute(&mut ctx, &["gpt-5.5".to_string()])
        .expect("Codex literal must produce Ok(())");

    let expected = "gpt-5.5".to_string();
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetModelOverride(s)) => assert_eq!(s, &expected),
        Some(other) => panic!("unexpected CommandEffect variant: {other:?}"),
        None => panic!("WRITE path must stash a CommandEffect::SetModelOverride"),
    }

    let events = drain_tui_events(&mut rx);
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, TuiEvent::ModelChanged(model) if model == &expected)),
        "Codex literal must emit TuiEvent::ModelChanged({expected})"
    );
}

#[test]
fn model_handler_execute_with_codex_alias_sets_effect() {
    let (mut ctx, _rx) = make_model_ctx(Some(codex_snapshot("gpt-5.4")));
    let h = ModelHandler;
    h.execute(&mut ctx, &["mini".to_string()])
        .expect("Codex alias must produce Ok(())");

    // Asserted against the configured value rather than a literal. This test
    // pinned `gpt-5.4-mini` and broke when the Codex model defaults began
    // sourcing from the shipped config.toml — the literal was a second copy of
    // a default, which is the exact drift that change removed. Resolving the
    // alias the same way production does keeps the test about the alias
    // mechanism instead of about which model happens to be current.
    let expected = archon_core::config::OpenAiCodexModelsConfig::default().mini;
    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetModelOverride(s)) => assert_eq!(s, &expected),
        Some(other) => panic!("unexpected CommandEffect variant: {other:?}"),
        None => panic!("WRITE path must stash a CommandEffect::SetModelOverride"),
    }
}

#[test]
fn model_handler_execute_accepts_generic_provider_model_id() {
    let (mut ctx, _rx) = make_model_ctx(Some(provider_snapshot("deepseek-v4-flash")));
    let h = ModelHandler;
    h.execute(&mut ctx, &["deepseek-v4-pro[1m]".to_string()])
        .expect("generic provider model ids must be accepted");

    match ctx.pending_effect.as_ref() {
        Some(CommandEffect::SetModelOverride(s)) => assert_eq!(s, "deepseek-v4-pro[1m]"),
        Some(other) => panic!("unexpected CommandEffect variant: {other:?}"),
        None => panic!("WRITE path must stash a CommandEffect::SetModelOverride"),
    }
}
