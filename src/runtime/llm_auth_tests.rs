//! `[api] auth` (#228): an endpoint declared keyless builds a client with no
//! credential; the default keeps today's refusal; a credential that is
//! present is used either way.

use archon_core::config::{ApiAuth, ArchonConfig};
use archon_core::env_vars::{ArchonEnvVars, load_env_vars_from};
use archon_llm::auth::{AuthProvider, resolve_auth_with_keys};

use super::resolve_configured_auth;

fn env(pairs: &[(&str, &str)]) -> ArchonEnvVars {
    load_env_vars_from(
        &pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
    )
}

fn config(toml_text: &str) -> ArchonConfig {
    toml::from_str(toml_text).expect("config parses")
}

const KEYLESS: &str = "[api]\nbase_url = \"http://127.0.0.1:4000/v1/messages\"\nauth = \"none\"\n";
const PROXY_ONLY: &str = "[api]\nbase_url = \"http://127.0.0.1:4000/v1/messages\"\n";

#[test]
fn auth_parses_from_config_and_defaults_to_required() {
    assert_eq!(config(KEYLESS).api.auth, ApiAuth::None);
    assert_eq!(config(PROXY_ONLY).api.auth, ApiAuth::Required);
    assert_eq!(ArchonConfig::default().api.auth, ApiAuth::Required);
    assert!(toml::from_str::<ArchonConfig>("[api]\nauth = \"maybe\"\n").is_err());
}

#[test]
fn a_keyless_endpoint_builds_a_client_with_no_key() {
    let config = config(KEYLESS);

    let auth = resolve_configured_auth(&config.api, &env(&[]), None)
        .expect("a keyless endpoint needs no credential");

    assert!(matches!(auth, AuthProvider::ApiKey(_)), "{auth:?}");
    assert_eq!(auth.header(), ("x-api-key".to_string(), String::new()));
    let client = archon_llm::anthropic::AnthropicClient::new(
        auth,
        archon_llm::identity::IdentityProvider::new(
            archon_llm::identity::IdentityMode::Clean,
            "keyless-test".to_string(),
            "device".to_string(),
            String::new(),
        ),
        config.api.base_url.clone(),
    );
    assert_eq!(client.api_url(), "http://127.0.0.1:4000/v1/messages");
}

#[test]
fn without_auth_none_a_missing_key_is_the_same_error_as_before() {
    let config = config(PROXY_ONLY);

    let error = resolve_configured_auth(&config.api, &env(&[]), None).unwrap_err();
    let before = resolve_auth_with_keys(None, None, None, None).unwrap_err();

    assert_eq!(error.to_string(), before.to_string());
    assert!(
        error.to_string().contains("No credentials found"),
        "{error}"
    );
}

#[test]
fn a_key_that_is_present_is_still_used() {
    for text in [KEYLESS, PROXY_ONLY] {
        let config = config(text);

        let auth = resolve_configured_auth(
            &config.api,
            &env(&[("ANTHROPIC_API_KEY", "sk-ant-api-present")]),
            None,
        )
        .unwrap();

        assert_eq!(
            auth.header(),
            ("x-api-key".to_string(), "sk-ant-api-present".to_string()),
            "{text}"
        );
    }
}
