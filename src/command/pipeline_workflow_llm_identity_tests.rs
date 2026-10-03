//! The critic request identity (Issue 259 review): every setting that can
//! change an answer moves it; a secret never does and never appears in it.

use super::*;

fn identity(config: &ArchonConfig) -> String {
    request_identity(config, ProviderEndpointPolicy::ConfiguredOnly)
}

#[test]
fn the_output_ceiling_and_every_endpoint_change_the_identity() {
    let base = ArchonConfig::default();
    let id = identity(&base);
    assert_eq!(id.len(), 64, "a digest, never the settings in clear");
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(id, identity(&base.clone()), "stable for the same settings");

    let mut changed = base.clone();
    changed.api.max_tokens = Some(base.api.thinking_budget + 4096);
    assert_ne!(identity(&changed), id, "max_tokens");

    let mut changed = base.clone();
    changed.api.thinking_budget += 1;
    assert_ne!(
        identity(&changed),
        id,
        "the ceiling an unset max_tokens falls back to"
    );

    let mut changed = base.clone();
    changed.api.base_url = Some("https://gateway.example/v1/messages".into());
    assert_ne!(identity(&changed), id, "the configured endpoint");

    for provider in ["openai", "local"] {
        let mut selected = base.clone();
        selected.llm.provider = provider.into();
        let before = identity(&selected);
        assert_ne!(before, id, "the provider {provider}");
        let mut moved = selected.clone();
        moved.llm.openai.base_url = Some("https://other.example/v1".into());
        moved.llm.local.base_url = "http://127.0.0.2:11434".into();
        assert_ne!(identity(&moved), before, "{provider} endpoint");
    }
}

#[test]
fn a_secret_never_changes_or_enters_the_identity() {
    let mut base = ArchonConfig::default();
    base.llm.provider = "openai".into();
    let id = identity(&base);
    let mut keyed = base.clone();
    keyed.llm.openai.api_key = Some("sk-not-a-real-key".into());
    assert_eq!(identity(&keyed), id);
}
