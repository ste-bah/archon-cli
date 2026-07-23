use super::*;
use axum::body::Body;
use axum::http::{Request, header};
use tower::ServiceExt;

fn test_state(config: &WebConfig, token: Option<String>) -> AppState {
    let auth_required = token.is_some();
    let paths = WebRuntimePaths::default();
    AppState {
        token,
        api: WebApiState::from_server_config(
            config,
            auth_required,
            EffectivePolicySummary::default_safe(),
            paths.clone(),
        ),
        live: WebLiveManager::new(16),
        paths,
        chat_backend: None,
        ingest_jobs: ingest::new_job_store(),
    }
}

#[test]
fn runtime_paths_use_memory_database_file() {
    let paths = WebRuntimePaths::from_overrides(None, None);
    assert!(paths.memory_db.ends_with("memory.db"));
    assert!(!paths.memory_db.ends_with(".archon/memory"));
}

#[test]
fn runtime_paths_honor_explicit_memory_file() {
    let paths = WebRuntimePaths::from_overrides(Some("/tmp/custom-memory.db"), None);
    assert_eq!(paths.memory_db, PathBuf::from("/tmp/custom-memory.db"));
}

#[test]
fn nonlocal_bind_without_token_is_rejected_by_default() {
    let config = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        ..WebConfig::default()
    };
    let err = validate_bind_auth(&config, &None, false).unwrap_err();
    assert!(err.to_string().contains("refusing to bind"));
}

#[test]
fn nonlocal_bind_with_blank_token_is_rejected_by_default() {
    let config = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        ..WebConfig::default()
    };
    let err = validate_bind_auth(&config, &Some("   ".to_string()), false).unwrap_err();
    assert!(err.to_string().contains("refusing to bind"));
}

#[test]
fn nonlocal_bind_with_token_is_allowed() {
    let config = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        ..WebConfig::default()
    };
    let token = validate_bind_auth(&config, &Some("secret-token".to_string()), false).unwrap();
    assert_eq!(token.as_deref(), Some("secret-token"));
}

#[test]
fn nonlocal_bind_without_token_requires_explicit_cli_unsafe_override() {
    let config = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        ..WebConfig::default()
    };
    let token = validate_bind_auth(&config, &None, true).unwrap();
    assert!(token.is_none());
}

#[tokio::test]
async fn graceful_server_shutdown_times_out_for_stalled_request() {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let error = server_shutdown::await_shutdown(
        std::future::pending::<std::io::Result<()>>(),
        async {},
        shutdown_tx,
    )
    .await
    .expect_err("stalled active request must not block server shutdown forever");

    assert!(error.to_string().contains("timed out"), "{error:#}");
    assert_eq!(shutdown_rx.await, Ok(()));
}

#[tokio::test]
async fn chat_submit_requires_bearer_auth_when_token_is_configured() {
    let config = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        open_browser: false,
        ..WebConfig::default()
    };
    let app = build_app(&config, test_state(&config, Some("secret-token".into())));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat/submit")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"message":"hello","attachments":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn chat_submit_body_limit_is_configurable() {
    let config = WebConfig {
        open_browser: false,
        max_body_bytes: 16,
        ..WebConfig::default()
    };
    let app = build_app(&config, test_state(&config, None));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chat/submit")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"message":"this body is too large","attachments":[]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
