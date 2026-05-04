use std::collections::BTreeMap;

use archon_llm::providers::codex::spoof::{
    CodexManifestConfig, CodexProviderConfig, Manifest, ResolvedSource, SpoofConfig, SpoofError,
    bundled_manifest, fetch_manifest, resolve,
};
use serial_test::serial;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn manifest_json(originator: &str, user_agent: &str) -> String {
    format!(
        r#"{{
          "schema_version": 1,
          "codex_cli_version": "0.34.1",
          "openclaw_version": "2026.5.1-beta.2",
          "spoof": {{
            "originator": "{originator}",
            "user_agent": "{user_agent}",
            "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
            "openai_beta": "responses=experimental",
            "extra_headers": {{}}
          }},
          "compatible_through": "2026-09-30",
          "minimum_archon_version": "0.1.0"
        }}"#
    )
}

fn clear_codex_env() {
    for key in [
        "ARCHON_CODEX_DISABLED",
        "ARCHON_CODEX_ORIGINATOR",
        "ARCHON_CODEX_USER_AGENT",
        "ARCHON_CODEX_CLIENT_ID",
        "ARCHON_CODEX_BETA",
        "ARCHON_CODEX_FETCH_URL",
        "ARCHON_CODEX_SPOOF_ALLOW_MIXED",
    ] {
        unsafe {
            std::env::remove_var(key);
        }
    }
}

#[test]
fn bundled_manifest_is_valid_and_legal() {
    let manifest = bundled_manifest().expect("bundled manifest validates");

    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.spoof.originator, "openclaw");
    assert!(manifest.spoof.user_agent.starts_with("openclaw/"));
}

#[test]
fn legal_guardrail_rejects_openai_product_impersonation() {
    for value in ["ChatGPT-Mac/1", "chatgpt/1", "OpenAI-Codex/1", "openai/1"] {
        let mut spoof = SpoofConfig::default();
        spoof.user_agent = value.into();
        assert!(matches!(
            spoof.validate(),
            Err(SpoofError::ImpersonationRejected { .. })
        ));
    }
}

#[test]
fn extra_headers_reject_reserved_names_and_impersonating_values() {
    let mut spoof = SpoofConfig::default();
    spoof.extra_headers = BTreeMap::from([("Authorization".into(), "x".into())]);
    assert!(matches!(spoof.validate(), Err(SpoofError::Validation(_))));

    let mut spoof = SpoofConfig::default();
    spoof.extra_headers = BTreeMap::from([("x-client".into(), "OpenAI-Codex/1".into())]);
    assert!(matches!(
        spoof.validate(),
        Err(SpoofError::ImpersonationRejected { .. })
    ));
}

#[tokio::test]
#[serial]
async fn env_tier_is_atomic_with_bundled_fallbacks() {
    clear_codex_env();
    unsafe {
        std::env::set_var("ARCHON_CODEX_ORIGINATOR", "envorigin");
    }

    let resolution = resolve(&CodexProviderConfig::default(), &reqwest::Client::new())
        .await
        .expect("resolve env tier");

    assert_eq!(resolution.primary_source, ResolvedSource::EnvVar);
    assert_eq!(resolution.config.originator, "envorigin");
    assert!(resolution.config.user_agent.starts_with("openclaw/"));
    assert!(resolution.per_field_fallbacks.contains_key("user_agent"));
    clear_codex_env();
}

#[tokio::test]
#[serial]
async fn config_tier_wins_when_originator_and_user_agent_are_set() {
    clear_codex_env();
    let mut cfg = CodexProviderConfig::default();
    cfg.spoof.originator = Some("cfgorigin".into());
    cfg.spoof.user_agent = Some("cfgagent/1".into());

    let resolution = resolve(&cfg, &reqwest::Client::new())
        .await
        .expect("resolve config tier");

    assert_eq!(resolution.primary_source, ResolvedSource::ConfigToml);
    assert_eq!(resolution.config.originator, "cfgorigin");
    assert_eq!(resolution.config.user_agent, "cfgagent/1");
}

#[tokio::test]
#[serial]
async fn disabled_env_var_blocks_resolution() {
    clear_codex_env();
    unsafe {
        std::env::set_var("ARCHON_CODEX_DISABLED", "1");
    }
    let err = resolve(&CodexProviderConfig::default(), &reqwest::Client::new())
        .await
        .expect_err("disabled");
    assert!(matches!(err, SpoofError::Disabled));
    clear_codex_env();
}

#[tokio::test]
async fn fetch_manifest_writes_and_reuses_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/codex-compat.json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(manifest_json("fetched", "fetched-agent/1")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let cache = tempfile::tempdir().expect("tempdir");
    let cfg = CodexManifestConfig {
        fetch_url: format!("{}/codex-compat.json", server.uri()),
        ttl_seconds: 21_600,
        cache_dir: cache.path().to_string_lossy().to_string(),
    };

    let first = fetch_manifest(&reqwest::Client::new(), &cfg)
        .await
        .expect("fetch");
    let second = fetch_manifest(&reqwest::Client::new(), &cfg)
        .await
        .expect("cache");

    assert_eq!(first.spoof.originator, "fetched");
    assert_eq!(second.spoof.user_agent, "fetched-agent/1");
    assert!(cache.path().join("codex-compat-cache.json").exists());
}

#[test]
fn future_schema_is_rejected() {
    let mut manifest: Manifest =
        serde_json::from_str(&manifest_json("future", "future-agent/1")).expect("manifest");
    manifest.schema_version = 99;

    assert!(matches!(
        manifest.validate(),
        Err(SpoofError::SchemaVersionUnsupported(99))
    ));
}
