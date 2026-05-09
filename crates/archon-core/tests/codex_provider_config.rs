use archon_core::config::ArchonConfig;

#[test]
fn archon_config_defaults_codex_provider_enabled() {
    let cfg = ArchonConfig::default();

    assert!(cfg.providers.openai_codex.enabled);
    assert_eq!(cfg.providers.openai_codex.runtime, "direct");
    assert!(!cfg.providers.openai_codex.direct_fallback);
    assert_eq!(
        cfg.providers.openai_codex.app_server_discovery_timeout_ms,
        2_500
    );
    assert_eq!(
        cfg.providers.openai_codex.app_server_model_catalog,
        vec!["gpt-5.5".to_string(), "gpt-5.4".to_string()]
    );
    assert_eq!(cfg.providers.openai_codex.manifest.ttl_seconds, 21_600);
}

#[test]
fn archon_config_parses_openai_codex_provider_section() {
    let cfg: ArchonConfig = toml::from_str(
        r#"
        [providers.openai-codex]
        enabled = true
        runtime = "auto"
        direct_fallback = true
        app_server_url = "http://127.0.0.1:11434/codex"
        app_server_discovery_timeout_ms = 750
        app_server_model_catalog = ["gpt-5.5", "gpt-5.4-mini"]

        [providers.openai-codex.spoof]
        originator = "cfgorigin"
        user_agent = "cfgagent/1"
        client_id = "app_EMoamEEZ73f0CkXaXp7hrann"
        openai_beta = "responses=experimental"

        [providers.openai-codex.spoof.extra_headers]
        x-test = "one"

        [providers.openai-codex.manifest]
        fetch_url = "https://example.invalid/codex-compat.json"
        ttl_seconds = 10
        cache_dir = "/tmp/archon-codex-cache"
        "#,
    )
    .expect("config parses");

    assert_eq!(cfg.providers.openai_codex.runtime, "auto");
    assert!(cfg.providers.openai_codex.direct_fallback);
    assert_eq!(
        cfg.providers.openai_codex.app_server_discovery_timeout_ms,
        750
    );
    assert_eq!(
        cfg.providers.openai_codex.app_server_url.as_deref(),
        Some("http://127.0.0.1:11434/codex")
    );
    assert_eq!(
        cfg.providers.openai_codex.app_server_model_catalog,
        vec!["gpt-5.5".to_string(), "gpt-5.4-mini".to_string()]
    );
    assert_eq!(
        cfg.providers.openai_codex.spoof.originator.as_deref(),
        Some("cfgorigin")
    );
    assert_eq!(cfg.providers.openai_codex.manifest.ttl_seconds, 10);
    assert_eq!(
        cfg.providers
            .openai_codex
            .spoof
            .extra_headers
            .get("x-test")
            .map(String::as_str),
        Some("one")
    );
}
