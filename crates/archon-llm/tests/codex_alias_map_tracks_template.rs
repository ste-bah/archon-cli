//! `CodexAliasMap::default()` must equal `[models.openai-codex]` in the shipped
//! `config.toml`.
//!
//! This map is the fallback for callers that cannot supply operator config —
//! notably `build_llm_provider`, which takes the flat `LlmConfig` and so has no
//! `[models.openai-codex]` slice to read. It previously carried its own literals
//! (`opus = gpt-5.4`, `sonnet = gpt-5.5`) which disagreed both with the template
//! *and* with `OpenAiCodexModelsConfig::to_alias_map()`, where `opus` and
//! `sonnet` both map to the flagship. Two fallbacks, two different answers.
//!
//! `archon-llm` sits below `archon-core`, so it cannot assert against
//! `OpenAiCodexModelsConfig::default()` directly. It asserts against the same
//! file instead — read from disk here, embedded in the impl — which is what
//! makes the two crates' independent readings verifiably the same values.

use std::path::{Path, PathBuf};

use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::codex::CodexAliasMap;
use archon_llm::providers::codex::client::CodexProvider;
use archon_llm::providers::codex::spoof_default::SpoofConfig;

fn template_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config.toml")
}

fn codex_model(key: &str) -> String {
    let path = template_path();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
        .parse::<toml::Value>()
        .expect("shipped config.toml must be valid TOML")
        .get("models")
        .and_then(|models| models.get("openai-codex"))
        .and_then(|slice| slice.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("config.toml is missing models.openai-codex.{key}"))
        .to_string()
}

#[test]
fn codex_defaults_track_the_template() {
    let map = CodexAliasMap::default();
    let flagship = codex_model("default");

    assert_eq!(map.opus, flagship);
    assert_eq!(map.sonnet, flagship);
    assert_eq!(map.haiku, codex_model("mini"));
    assert_eq!(map.codex, codex_model("codex"));
}

/// Build a provider whose alias map names models this build has no hardcoded
/// knowledge of, so a test that passes cannot be passing by coincidence with the
/// known-model table.
fn provider_with_aliases(aliases: CodexAliasMap) -> CodexProvider {
    CodexProvider::new(
        PathBuf::from("/tmp/archon-test-codex-alias-map.json"),
        SpoofConfig::default(),
        reqwest::Client::new(),
    )
    .expect("provider")
    .with_alias_map(aliases)
}

fn unknown_aliases() -> CodexAliasMap {
    CodexAliasMap {
        opus: "gpt-9.9-flagship".into(),
        sonnet: "gpt-9.9-flagship".into(),
        haiku: "gpt-9.9-mini".into(),
        codex: "gpt-9.9-codex".into(),
    }
}

/// `resolve_request_model` falls back to `models().first()` for a request with
/// no model. That head slot is therefore the effective default, and it must come
/// from config — it previously came from a literal `gpt-5.5`.
#[test]
fn models_lead_with_the_configured_flagship() {
    let provider = provider_with_aliases(unknown_aliases());

    assert_eq!(
        provider.models().first().expect("non-empty").id,
        "gpt-9.9-flagship"
    );
}

#[test]
fn empty_request_model_resolves_to_the_configured_flagship() {
    let provider = provider_with_aliases(unknown_aliases());
    let mut request = LlmRequest {
        model: String::new(),
        ..LlmRequest::default()
    };

    provider.resolve_request_model(&mut request);

    assert_eq!(request.model, "gpt-9.9-flagship");
}

/// `context_window::for_model` resolves a window by finding the id in `models()`.
/// A configured model absent from that list loses its window, so every alias the
/// operator can select must be enumerated.
#[test]
fn every_configured_alias_appears_in_models() {
    let aliases = unknown_aliases();
    let provider = provider_with_aliases(aliases.clone());
    let listed: Vec<String> = provider
        .models()
        .into_iter()
        .map(|model| model.id)
        .collect();

    for (tier, id) in [
        ("opus", &aliases.opus),
        ("sonnet", &aliases.sonnet),
        ("haiku", &aliases.haiku),
        ("codex", &aliases.codex),
    ] {
        assert!(
            listed.contains(id),
            "tier `{tier}` resolves to `{id}`, which models() does not list: {listed:?}"
        );
    }
}

/// Leading with configured models must not drop the known catalog — those ids
/// carry the real context windows and feed model enumeration in the UI.
#[test]
fn known_catalog_survives_alongside_configured_models() {
    let provider = provider_with_aliases(unknown_aliases());
    let listed: Vec<String> = provider
        .models()
        .into_iter()
        .map(|model| model.id)
        .collect();

    for known in ["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex"] {
        assert!(
            listed.contains(&known.to_string()),
            "models() dropped `{known}`: {listed:?}"
        );
    }
}

/// A configured model that is also in the known table must keep its real window
/// rather than the conservative unknown-model fallback, and must not be listed
/// twice.
#[test]
fn configured_known_model_keeps_its_context_window_and_is_not_duplicated() {
    let provider = provider_with_aliases(CodexAliasMap {
        opus: "gpt-5.5".into(),
        sonnet: "gpt-5.5".into(),
        haiku: "gpt-5.4-mini".into(),
        codex: "gpt-5.3-codex".into(),
    });
    let models = provider.models();

    let flagship: Vec<_> = models
        .iter()
        .filter(|model| model.id == "gpt-5.5")
        .collect();
    assert_eq!(flagship.len(), 1, "gpt-5.5 listed more than once");
    assert_eq!(flagship[0].context_window, 1_050_000);
}

/// The bug this replaces was a stale *literal*, so pin that no tier still
/// answers with a model absent from the template.
#[test]
fn no_tier_resolves_to_a_model_outside_the_template() {
    let map = CodexAliasMap::default();
    let known = [
        codex_model("default"),
        codex_model("codex"),
        codex_model("mini"),
    ];

    for (tier, model) in [
        ("opus", &map.opus),
        ("sonnet", &map.sonnet),
        ("haiku", &map.haiku),
        ("codex", &map.codex),
    ] {
        assert!(
            known.contains(model),
            "tier `{tier}` resolves to `{model}`, which is not in [models.openai-codex]"
        );
    }
}
