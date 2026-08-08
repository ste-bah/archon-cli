//! Tests for the terminal gate.
//!
//! The router-level cases matter more than the predicate ones: the promise is
//! that the *route does not exist*, and only driving `build_app` can show that.
//! A predicate test would still pass if someone registered the route
//! unconditionally and checked inside the handler.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;
use crate::web::api::{WebApiState, WebPolicySummary};
use crate::web::server::build_app;
use crate::web::{AppState, WebRuntimeHandles, WebRuntimePaths, agents, board, ingest, live};

const TERMINAL_PATH: &str = "/api/terminal/ws";

fn policy(allow_web_terminal: bool) -> EffectivePolicySummary {
    EffectivePolicySummary {
        web: WebPolicySummary {
            allow_web_terminal,
            ..WebPolicySummary::default_safe()
        },
        ..EffectivePolicySummary::default_safe()
    }
}

fn config(bind_address: &str) -> WebConfig {
    WebConfig {
        bind_address: bind_address.to_string(),
        open_browser: false,
        ..WebConfig::default()
    }
}

fn state(config: &WebConfig, policy: EffectivePolicySummary) -> AppState {
    let paths = WebRuntimePaths::default();
    AppState {
        token: None,
        api: WebApiState::from_server_config(config, false, policy, paths.clone()),
        live: live::WebLiveManager::new(16),
        paths,
        chat_backend: None,
        ingest_jobs: ingest::new_job_store(),
        handles: WebRuntimeHandles::default(),
        agents: agents::WebAgentObserver::new(),
        board: board::WebBoardStore::new(),
        attached: false,
    }
}

/// A plain GET on the WebSocket path. When the route exists this fails the
/// upgrade with 400/426; when it does not exist it is a 404. Those are
/// distinguishable, which is the whole point.
async fn probe(config: WebConfig, policy: EffectivePolicySummary) -> StatusCode {
    let app = build_app(&config, state(&config, policy));
    app.oneshot(
        Request::builder()
            .uri(TERMINAL_PATH)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

#[test]
fn unavailable_without_the_policy_flag() {
    assert!(!is_available(&config("127.0.0.1"), &policy(false)));
}

#[test]
fn unavailable_on_a_non_loopback_bind_even_with_the_flag() {
    assert!(!is_available(&config("0.0.0.0"), &policy(true)));
    assert!(!is_available(&config("192.168.1.10"), &policy(true)));
}

#[test]
fn available_on_loopback_with_the_flag() {
    assert!(is_available(&config("127.0.0.1"), &policy(true)));
    assert!(is_available(&config("::1"), &policy(true)));
}

#[test]
fn mutating_actions_alone_do_not_open_a_shell() {
    let mut allowed = EffectivePolicySummary::default_safe();
    allowed.web.allow_mutating_actions = true;
    allowed.web.allow_file_uploads = true;
    assert!(!is_available(&config("127.0.0.1"), &allowed));
}

#[tokio::test]
async fn route_is_absent_with_the_policy_flag_off() {
    assert_eq!(
        probe(config("127.0.0.1"), policy(false)).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn route_is_absent_on_a_non_loopback_bind() {
    assert_eq!(
        probe(config("0.0.0.0"), policy(true)).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn route_exists_on_loopback_with_the_flag() {
    // Not 404: the route is registered and the upgrade handshake itself is
    // what rejects a plain GET.
    let status = probe(config("127.0.0.1"), policy(true)).await;
    assert_ne!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn status_feature_flag_tracks_the_route() {
    let cfg = config("127.0.0.1");
    let api =
        WebApiState::from_server_config(&cfg, false, policy(true), WebRuntimePaths::default());
    assert!(api.status().features.terminal);

    let cfg = config("0.0.0.0");
    let api = WebApiState::from_server_config(&cfg, true, policy(true), WebRuntimePaths::default());
    assert!(
        !api.status().features.terminal,
        "a non-loopback bind must not advertise a terminal it does not serve"
    );
}

#[test]
fn resize_control_frames_parse() {
    let parsed: TerminalControl =
        serde_json::from_str(r#"{"type":"resize","cols":120,"rows":40}"#).expect("parse");
    let TerminalControl::Resize { cols, rows } = parsed;
    assert_eq!((cols, rows), (120, 40));
}

#[test]
fn cross_origin_upgrades_are_refused() {
    let cfg = config("127.0.0.1");
    let state = state(&cfg, policy(true));

    let mut headers = HeaderMap::new();
    headers.insert(ORIGIN, "http://evil.example".parse().unwrap());
    assert!(!origin_is_local(&state, &headers));

    headers.insert(
        ORIGIN,
        format!("http://127.0.0.1:{}", cfg.port).parse().unwrap(),
    );
    assert!(origin_is_local(&state, &headers));

    // No Origin at all is a non-browser client; browsers always send one.
    assert!(origin_is_local(&state, &HeaderMap::new()));
}
