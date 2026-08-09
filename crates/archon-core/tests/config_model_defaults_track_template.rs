//! The compiled-in model defaults must equal `[models.*]` in the shipped `config.toml`.
//!
//! These two had silently drifted. The template said `claude-opus-5` and
//! `gpt-5.6-sol`; the `Default` impls still said `claude-opus-4-8` and `gpt-5.5`.
//! Because the config structs are `#[serde(default)]`, any installation whose
//! `config.toml` omitted a `[models.*]` key got the stale Rust value rather than
//! the shipped one — with nothing in the logs to say so.
//!
//! The `Default` impls now read the embedded template, so the two are the same
//! bytes by construction. This suite is the guard on that: it re-reads
//! `config.toml` from disk rather than through the same `include_str!`, so a
//! change that broke the embedding (wrong relative path, wrong key) fails here
//! instead of shipping.

use std::path::{Path, PathBuf};

use archon_core::config::{
    AnthropicModelsConfig, CodexProviderConfig, OpenAiCodexModelsConfig, resolve_anthropic_model,
    resolve_codex_model,
};

fn template_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config.toml")
}

fn template() -> toml::Value {
    let path = template_path();
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
        .parse::<toml::Value>()
        .expect("shipped config.toml must be valid TOML")
}

/// Read `models.<provider>.<key>`, failing with the full path when absent.
fn model(template: &toml::Value, provider: &str, key: &str) -> String {
    template
        .get("models")
        .and_then(|models| models.get(provider))
        .and_then(|slice| slice.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("config.toml is missing models.{provider}.{key}"))
        .to_string()
}

#[test]
fn anthropic_defaults_track_the_shipped_template() {
    let template = template();
    let defaults = AnthropicModelsConfig::default();

    assert_eq!(defaults.opus, model(&template, "anthropic", "opus"));
    assert_eq!(defaults.sonnet, model(&template, "anthropic", "sonnet"));
    assert_eq!(defaults.haiku, model(&template, "anthropic", "haiku"));
}

#[test]
fn codex_defaults_track_the_shipped_template() {
    let template = template();
    let defaults = OpenAiCodexModelsConfig::default();

    assert_eq!(
        defaults.default,
        model(&template, "openai-codex", "default")
    );
    assert_eq!(defaults.codex, model(&template, "openai-codex", "codex"));
    assert_eq!(defaults.mini, model(&template, "openai-codex", "mini"));
}

/// The `Default` impls panic on a missing template key. Calling both of them
/// proves every key they reach for is actually present, so that panic is
/// unreachable in a shipped binary rather than merely unlikely.
#[test]
fn template_defaults_cover_every_model_key() {
    let anthropic = AnthropicModelsConfig::default();
    let codex = OpenAiCodexModelsConfig::default();

    for value in [
        &anthropic.opus,
        &anthropic.sonnet,
        &anthropic.haiku,
        &codex.default,
        &codex.codex,
        &codex.mini,
    ] {
        assert!(!value.trim().is_empty(), "model default resolved to empty");
    }
}

/// The alias resolvers are what production actually calls. Pinning them against
/// the template closes the gap between "the struct holds the right string" and
/// "asking for `sonnet` returns it".
#[test]
fn alias_resolvers_return_template_models_under_defaults() {
    let template = template();
    let anthropic = AnthropicModelsConfig::default();
    let codex = OpenAiCodexModelsConfig::default();

    assert_eq!(
        resolve_anthropic_model("sonnet", &anthropic),
        model(&template, "anthropic", "sonnet")
    );
    assert_eq!(
        resolve_codex_model("default", &codex),
        model(&template, "openai-codex", "default")
    );
    // Empty input is documented to mean `default`; it must not fall back to a
    // hardcoded id.
    assert_eq!(
        resolve_codex_model("", &codex),
        model(&template, "openai-codex", "default")
    );
}

/// The app-server catalog gates which models that transport will offer. Its
/// hardcoded value had gone stale in the same way — `["gpt-5.5", "gpt-5.4"]`
/// against a template listing five ids — so an operator who omitted the key got
/// a catalog with no 5.6 model in it.
#[test]
fn app_server_model_catalog_tracks_the_shipped_template() {
    let template = template();
    let expected: Vec<String> = template
        .get("providers")
        .and_then(|providers| providers.get("openai-codex"))
        .and_then(|slice| slice.get("app_server_model_catalog"))
        .and_then(toml::Value::as_array)
        .expect("config.toml is missing providers.openai-codex.app_server_model_catalog")
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .expect("catalog entries must be strings")
                .to_string()
        })
        .collect();

    assert_eq!(
        CodexProviderConfig::default().app_server_model_catalog,
        expected
    );
}

/// The catalog must contain the default model, or the app-server transport
/// cannot serve the very model the rest of the config points at.
#[test]
fn app_server_catalog_contains_the_default_codex_model() {
    let catalog = CodexProviderConfig::default().app_server_model_catalog;
    let default_model = OpenAiCodexModelsConfig::default().default;

    assert!(
        catalog.contains(&default_model),
        "app_server_model_catalog {catalog:?} does not contain the default model `{default_model}`"
    );
}

/// `to_alias_map()` is the hand-off into the provider. Its tier mapping must
/// carry template values through unchanged — this is the exact path whose
/// breakage surfaced as a session reporting `gpt-5.5` while config said
/// `gpt-5.6-sol`.
#[test]
fn codex_alias_map_carries_template_models() {
    let template = template();
    let map = OpenAiCodexModelsConfig::default().to_alias_map();

    let flagship = model(&template, "openai-codex", "default");
    assert_eq!(map.opus, flagship);
    assert_eq!(map.sonnet, flagship);
    assert_eq!(map.haiku, model(&template, "openai-codex", "mini"));
    assert_eq!(map.codex, model(&template, "openai-codex", "codex"));
}
