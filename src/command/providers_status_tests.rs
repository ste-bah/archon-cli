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
    config.providers.openai_codex.app_server_url = Some("http://127.0.0.1:11434/codex".into());

    let body = render_provider_status_with_env_and_config(
        Some("openai-codex"),
        &ProviderStatusEnv::default(),
        &config,
    );

    assert!(body.contains("app_server"));
    assert!(body.contains("app-server"));
    assert!(body.contains("unavailable"));
    assert!(body.contains("adapter-pending"));
}

#[test]
fn codex_status_persists_app_server_discovery_metadata() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.providers.openai_codex.runtime = "auto".into();
    config.providers.openai_codex.direct_fallback = true;
    config.providers.openai_codex.app_server_url = Some("http://127.0.0.1:11434/codex".into());
    let status = status_from_descriptor(
        archon_llm::providers::list_native()
            .iter()
            .find(|descriptor| descriptor.id == "openai-codex")
            .unwrap(),
        &ProviderStatusEnv {
            codex_oauth: true,
            ..ProviderStatusEnv::default()
        },
        &config,
    );

    assert_eq!(
        status.metadata_redacted_json["app_server_discovery"]["status"],
        "configured"
    );
    assert_eq!(
        status.metadata_redacted_json["app_server_discovery"]["endpoint_redacted"],
        "http://127.0.0.1:11434/[redacted]"
    );
    assert_eq!(
        status.metadata_redacted_json["codex_strategy"]["status_note"],
        "app-server:configured direct-fallback"
    );
}

#[test]
fn status_enrichment_preserves_existing_metadata() {
    let existing = serde_json::json!({
        "app_server_discovery": {"status": "configured"},
    });
    let incoming = serde_json::json!({
        "selected_profile_id": "codex-oauth",
    });

    let merged = merge_redacted_metadata(existing, incoming);

    assert_eq!(merged["app_server_discovery"]["status"], "configured");
    assert_eq!(merged["selected_profile_id"], "codex-oauth");
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
fn status_enrichment_adds_last_runtime_events_without_profile() {
    let db = test_db();
    archon_learning::runtime_events::insert_provider_runtime_event(
        &db,
        &archon_learning::runtime_models::ProviderRuntimeEventRecord::new(
            "provider-event-ok",
            "openai",
            "direct",
            "request_succeeded",
            "info",
            "2026-05-08T12:00:00Z",
        ),
    )
    .unwrap();
    archon_learning::runtime_events::insert_provider_runtime_event(
        &db,
        &archon_learning::runtime_models::ProviderRuntimeEventRecord::new(
            "provider-event-fail",
            "openai",
            "direct",
            "request_failed",
            "error",
            "2026-05-08T12:01:00Z",
        )
        .with_reason("auth_error"),
    )
    .unwrap();
    let mut statuses = vec![
        ProviderRuntimeStatus::new("openai", "direct").with_health(ProviderHealthStatus::Unknown),
    ];

    enrich_provider_statuses_from_db(&mut statuses, &db).unwrap();
    let body = render_provider_statuses(&statuses);

    assert_eq!(statuses[0].health, ProviderHealthStatus::Degraded);
    assert!(statuses[0].last_success_at.is_some());
    assert!(statuses[0].last_failure_at.is_some());
    assert_eq!(
        statuses[0].metadata_redacted_json["last_failure_event"],
        "provider-event-fail"
    );
    assert!(body.contains("last-failure:auth_error"));
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

#[test]
fn status_json_renders_redacted_snapshot() {
    let status = ProviderRuntimeStatus::new("anthropic", "direct")
        .with_display_name("Anthropic")
        .with_model("claude-sonnet-4-6")
        .with_identity_status(ProviderIdentityStatus::Spoof)
        .with_health(ProviderHealthStatus::Healthy)
        .with_redacted_json(serde_json::json!({
            "authorization": "Bearer secret",
            "safe": "kept"
        }));

    let body = render_provider_statuses_json(&[status]).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(parsed["provider_count"], 1);
    assert_eq!(parsed["providers"][0]["provider_id"], "anthropic");
    assert_eq!(parsed["providers"][0]["health"], "healthy");
    assert_eq!(parsed["providers"][0]["identity_status"], "spoof");
    assert_eq!(
        parsed["providers"][0]["metadata_redacted_json"]["authorization"],
        "[redacted]"
    );
    assert_eq!(
        parsed["providers"][0]["metadata_redacted_json"]["safe"],
        "kept"
    );
}
