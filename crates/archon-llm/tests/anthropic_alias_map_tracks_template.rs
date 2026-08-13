//! `AnthropicAliasMap::default()` must equal `[models.anthropic]` in the shipped
//! `config.toml`.
//!
//! This map is the fallback for callers that cannot supply operator config.
//! Every production path in `src/runtime/llm.rs` passes
//! `.with_alias_map(models_cfg.anthropic.to_alias_map())`, so the default is
//! reachable only from tests and from any future caller that forgets — which is
//! precisely how it went stale without anyone noticing.
//!
//! It had drifted to `claude-opus-4-8` while the template said
//! `claude-opus-5`. `archon-core` had already fixed this exact class of bug on
//! its side, by reading the template at runtime rather than carrying literals,
//! and its comment even names `claude-opus-4-8` as the stale value. But
//! `archon-core` sits *above* `archon-llm`, so it cannot lend this crate that
//! reader; the literal stayed, and unlike the Codex map next door it had no
//! guard test.
//!
//! The consequence is not cosmetic. An alias resolves to a model id, and the id
//! selects the prompt-cache minimum: `claude-opus-4-8` asks for a 1,024-token
//! prefix where `claude-opus-5` needs 512, so a stale alias quietly changes when
//! caching starts.
//!
//! `archon-llm` cannot assert against `AnthropicModelsConfig::default()`
//! directly, so it asserts against the same file — read from disk here,
//! embedded as literals in the impl — which is what makes the two crates'
//! independent readings verifiably identical.

use std::path::{Path, PathBuf};

use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::LlmProvider;
use archon_llm::providers::anthropic::{AnthropicAliasMap, AnthropicProvider};
use archon_llm::types::Secret;

fn template_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config.toml")
}

fn anthropic_model(key: &str) -> String {
    let path = template_path();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
        .parse::<toml::Value>()
        .expect("shipped config.toml must be valid TOML")
        .get("models")
        .and_then(|models| models.get("anthropic"))
        .and_then(|slice| slice.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("config.toml is missing models.anthropic.{key}"))
        .to_string()
}

#[test]
fn anthropic_defaults_track_the_template() {
    let map = AnthropicAliasMap::default();

    assert_eq!(map.opus, anthropic_model("opus"));
    assert_eq!(map.sonnet, anthropic_model("sonnet"));
    assert_eq!(map.haiku, anthropic_model("haiku"));
}

/// The bug this replaces was a stale *literal*, so pin that no tier answers with
/// a model absent from the template entirely.
#[test]
fn no_tier_resolves_to_a_model_outside_the_template() {
    let map = AnthropicAliasMap::default();
    let known = [
        anthropic_model("opus"),
        anthropic_model("sonnet"),
        anthropic_model("haiku"),
    ];

    for (tier, model) in [
        ("opus", &map.opus),
        ("sonnet", &map.sonnet),
        ("haiku", &map.haiku),
    ] {
        assert!(
            known.contains(model),
            "tier `{tier}` resolves to `{model}`, which is not in [models.anthropic]"
        );
    }
}

/// `models()` must lead with the configured opus, because its head is the
/// effective default.
///
/// `resolve_request_model` falls back to `models().first()` when a request
/// carries no model, so a hardcoded list does not merely go stale — it silently
/// overrides `[models.anthropic]`. This list led with `claude-opus-4-8` while
/// the template said `claude-opus-5`, and the two do not even share a cache
/// minimum: 1,024 tokens against 512.
#[test]
fn the_model_list_leads_with_the_configured_opus() {
    let client = AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        IdentityProvider::new(
            IdentityMode::Clean,
            "session".into(),
            "device".into(),
            String::new(),
        ),
        None,
    );
    let provider = AnthropicProvider::new(client).with_alias_map(AnthropicAliasMap::default());

    let models = provider.models();
    assert_eq!(
        models.first().map(|model| model.id.as_str()),
        Some(anthropic_model("opus").as_str()),
        "the head of models() is the fallback model, so it must be the configured opus"
    );

    // Every configured tier has to appear, not just the first.
    for key in ["opus", "sonnet", "haiku"] {
        let wanted = anthropic_model(key);
        assert!(
            models.iter().any(|model| model.id == wanted),
            "tier `{key}` resolves to `{wanted}`, which models() does not enumerate"
        );
    }
}

/// The reason a stale alias costs money rather than merely looking untidy: the
/// resolved id selects the prompt-cache minimum, and those differ sharply
/// between generations with no relationship to the version number.
#[test]
fn each_default_alias_resolves_to_a_model_the_cache_table_knows() {
    let map = AnthropicAliasMap::default();

    for (tier, model) in [
        ("opus", &map.opus),
        ("sonnet", &map.sonnet),
        ("haiku", &map.haiku),
    ] {
        let table = archon_llm::cache_models::ModelCacheTable::default();
        assert!(
            table.lookup(model).is_some(),
            "tier `{tier}` resolves to `{model}`, which the cache table does not \
             recognise — it would fall back to the conservative minimum and start \
             caching later than the model requires"
        );
    }
}
