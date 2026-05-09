use super::*;
use archon_learning::provider_auth_profiles::{
    ProviderAuthProfileRecord, insert_provider_auth_profile,
};

fn env_with(vars: &[&str]) -> ProviderStatusEnv {
    ProviderStatusEnv {
        env_vars: vars.iter().map(|name| name.to_string()).collect(),
        ..ProviderStatusEnv::default()
    }
}

fn test_db() -> DbInstance {
    let path = format!("/tmp/test-provider-status-{}.db", uuid::Uuid::new_v4());
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    db
}

#[test]
fn status_lists_local_provider_without_credentials() {
    let body = render_provider_status_with_env(Some("ollama"), &ProviderStatusEnv::default());

    assert!(body.contains("ollama"));
    assert!(body.contains("unknown-local"));
    assert!(body.contains("local"));
    assert!(body.contains("n/a"));
}

#[test]
fn status_marks_missing_credentials_for_remote_provider() {
    let body = render_provider_status_with_env(Some("openai"), &ProviderStatusEnv::default());

    assert!(body.contains("openai"));
    assert!(body.contains("missing-credentials"));
}

#[test]
fn status_marks_configured_env_provider_as_unknown_local() {
    let body = render_provider_status_with_env(Some("openai"), &env_with(&["OPENAI_API_KEY"]));

    assert!(body.contains("openai"));
    assert!(body.contains("unknown-local"));
}

#[test]
fn status_shows_anthropic_spoof_for_oauth_profile() {
    let env = ProviderStatusEnv {
        anthropic_oauth: true,
        ..ProviderStatusEnv::default()
    };
    let body = render_provider_status_with_env(Some("anthropic"), &env);

    assert!(body.contains("anthropic"));
    assert!(body.contains("spoof"));
}

#[test]
fn codex_status_uses_configured_runtime_not_oauth_presence() {
    let env = ProviderStatusEnv {
        codex_oauth: true,
        ..ProviderStatusEnv::default()
    };
    let mut config = archon_core::config::ArchonConfig::default();
    config.providers.openai_codex.runtime = "direct".into();

    let body = render_provider_status_with_env_and_config(Some("openai-codex"), &env, &config);

    assert!(body.contains("openai-codex"));
    assert!(body.contains("direct"));
    assert!(body.contains("custom"));
    assert!(!body.contains("app-server"));
}

#[test]
fn codex_status_shows_app_server_when_configured() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.providers.openai_codex.runtime = "app-server".into();

    let body = render_provider_status_with_env_and_config(
        Some("openai-codex"),
        &ProviderStatusEnv::default(),
        &config,
    );

    assert!(body.contains("app_server"));
    assert!(body.contains("app-server"));
}

#[test]
fn status_reports_empty_filter_result() {
    let body = render_provider_status_with_env(Some("missing-provider"), &env_with(&[]));

    assert!(body.contains("No provider matched"));
}

#[test]
fn status_snapshot_record_uses_redacted_status_metadata() {
    let status = ProviderRuntimeStatus::new("anthropic", "direct")
        .with_display_name("Anthropic")
        .with_model("claude-sonnet-4-6")
        .with_identity_status(ProviderIdentityStatus::Spoof)
        .with_health(ProviderHealthStatus::Healthy)
        .with_redacted_json(serde_json::json!({
            "authorization": "Bearer secret",
            "safe": "kept"
        }));

    let record = status_snapshot_record(&status);

    assert_eq!(record.provider_id, "anthropic");
    assert_eq!(record.identity_status, "spoof");
    assert_eq!(record.health, "healthy");
    assert_eq!(record.metadata_redacted_json["authorization"], "[redacted]");
    assert_eq!(record.metadata_redacted_json["safe"], "kept");
}

#[test]
fn status_enrichment_adds_selected_profile() {
    let db = test_db();
    insert_provider_auth_profile(
        &db,
        &ProviderAuthProfileRecord::new(
            "anthropic-oauth",
            "anthropic",
            "oauth",
            "archon_store",
            "2026-05-08T12:00:00Z",
        ),
    )
    .unwrap();
    let mut statuses = vec![
        ProviderRuntimeStatus::new("anthropic", "direct")
            .with_health(ProviderHealthStatus::MissingCredentials),
    ];

    enrich_provider_statuses_from_db(&mut statuses, &db).unwrap();

    assert_eq!(statuses[0].profile_id.as_deref(), Some("anthropic-oauth"));
    assert_eq!(statuses[0].health, ProviderHealthStatus::Unknown);
    assert_eq!(
        statuses[0].metadata_redacted_json["selected_profile_id"],
        "anthropic-oauth"
    );
}

#[test]
fn status_render_shows_recent_limit_notes() {
    let status = ProviderRuntimeStatus::new("openai-codex", "auto")
        .with_model("gpt-5.3-codex")
        .with_health(ProviderHealthStatus::Degraded)
        .with_rate_limits(vec![
            archon_llm::runtime::ProviderRateLimitWindow::new(
                "openai-codex",
                archon_llm::runtime::RateLimitWindowKind::Usage,
            )
            .with_used_percent(100.0),
        ]);

    let body = render_provider_statuses(&[status]);

    assert!(body.contains("limited:1"));
}
