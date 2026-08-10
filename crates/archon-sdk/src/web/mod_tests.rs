use super::api::EffectivePolicySummary;
use super::server::{build_app, validate_bind_auth};
use super::*;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, header};
use futures_util::StreamExt;
use tower::ServiceExt;

/// Anything longer than this and the stream is not streaming.
const FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
        handles: WebRuntimeHandles::default(),
        agents: agents::WebAgentObserver::new(),
        board: board::WebBoardStore::new(),
        attached: false,
    }
}

/// Read SSE frames until one carries a `data:` payload, and return it.
async fn first_sse_payload(app: Router, uri: &str) -> String {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let mut buffer = String::new();
    tokio::time::timeout(FRAME_TIMEOUT, async {
        while let Some(chunk) = body.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            if let Some(line) = buffer.lines().find(|line| line.starts_with("data:")) {
                return line.trim_start_matches("data:").trim().to_string();
            }
        }
        panic!("live stream ended before emitting a frame");
    })
    .await
    .expect("live stream frame")
}

#[tokio::test]
async fn live_stream_emits_events_after_the_supplied_cursor() {
    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let state = test_state(&config, None);
    let first = state.live.record("first", "first event");
    state.live.record("second", "second event");
    let app = build_app(&config, state);

    let payload = first_sse_payload(app, &format!("/api/live/stream?after={first}")).await;
    let snapshot: live::WebLiveSnapshot = serde_json::from_str(&payload).unwrap();

    assert_eq!(snapshot.events.len(), 1, "{payload}");
    assert_eq!(snapshot.events[0].event_type, "second");
    assert_eq!(snapshot.next_cursor, 3);
}

#[tokio::test]
async fn live_stream_emits_the_expired_shape_for_a_compacted_cursor() {
    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let mut state = test_state(&config, None);
    // One-event buffer: recording twice drops the first and marks the buffer
    // compacted, so cursor 0 is now unreachable.
    state.live = WebLiveManager::new(1);
    state.live.record("first", "first event");
    state.live.record("second", "second event");
    let app = build_app(&config, state);

    let payload = first_sse_payload(app, "/api/live/stream?after=0").await;
    let expired: live::WebLiveCursorExpired = serde_json::from_str(&payload).unwrap();

    assert!(expired.cursor_expired, "{payload}");
    assert_eq!(expired.recovery, "refetch full snapshot");
}

#[tokio::test]
async fn live_stream_requires_bearer_auth_when_token_is_configured() {
    let config = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        open_browser: false,
        ..WebConfig::default()
    };
    let app = build_app(&config, test_state(&config, Some("secret-token".into())));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/live/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agents_live_requires_bearer_auth_when_token_is_configured() {
    let config = WebConfig {
        bind_address: "0.0.0.0".to_string(),
        open_browser: false,
        ..WebConfig::default()
    };
    let app = build_app(&config, test_state(&config, Some("secret-token".into())));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/agents/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

async fn get_json(app: Router, uri: &str) -> serde_json::Value {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn agent_ids(snapshot: &serde_json::Value) -> Vec<String> {
    snapshot["agents"]
        .as_array()
        .expect("agents array")
        .iter()
        .map(|agent| agent["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn agents_live_reports_a_running_agent_and_drops_it_once_terminal() {
    use archon_tools::background_agents::{
        AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, new_result_slot,
    };

    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let state = test_state(&config, None);
    let agent_id = uuid::Uuid::new_v4();
    // Keep a clone of the status slot: the handle itself is moved into the
    // registry, and driving it terminal is the point of the test.
    let status = std::sync::Arc::new(std::sync::Mutex::new(AgentStatus::Running));
    BACKGROUND_AGENTS
        .register(BackgroundAgentHandle {
            agent_id,
            subagent_id: agent_id.to_string(),
            join_handle: None,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            spawned_at: std::time::SystemTime::now(),
            status: std::sync::Arc::clone(&status),
            result_slot: new_result_slot(),
        })
        .expect("register background agent");

    let running = get_json(build_app(&config, state.clone()), "/api/agents/live").await;
    assert!(
        agent_ids(&running).contains(&agent_id.to_string()),
        "{running}"
    );

    *status.lock().unwrap() = AgentStatus::Finished;

    let after = get_json(build_app(&config, state), "/api/agents/live").await;
    assert!(
        !agent_ids(&after).contains(&agent_id.to_string()),
        "terminal agent must stop being reported as live: {after}"
    );
}

#[tokio::test]
async fn agents_live_includes_task_manager_spawned_agents() {
    use archon_tools::task_manager::{TASK_MANAGER, TaskStatus};

    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let state = test_state(&config, None);
    // TaskCreate-spawned agents never reach BACKGROUND_AGENTS, so a projection
    // that reads only that registry silently reports them as nothing running.
    let task_id = TASK_MANAGER.create_task("web dashboard projection probe");
    TASK_MANAGER.set_status(&task_id, TaskStatus::Running);

    let snapshot = get_json(build_app(&config, state.clone()), "/api/agents/live").await;
    let entry = snapshot["agents"]
        .as_array()
        .expect("agents array")
        .iter()
        .find(|agent| agent["id"].as_str() == Some(task_id.as_str()))
        .unwrap_or_else(|| panic!("task agent missing from projection: {snapshot}"));
    assert_eq!(entry["kind"], "task");
    assert_eq!(entry["status"], "running");
    assert_eq!(entry["label"], "web dashboard projection probe");

    TASK_MANAGER.stop_task(&task_id).expect("stop probe task");
    let after = get_json(build_app(&config, state), "/api/agents/live").await;
    assert!(!agent_ids(&after).contains(&task_id), "{after}");
}

/// Issue #129: pipeline agents are registered at the spawn choke point like
/// everything else, so the dashboard shows them — under the id the pipeline
/// gave them, which reads far better than a UUID.
#[tokio::test]
async fn agents_live_includes_pipeline_spawned_agents() {
    use archon_tools::background_agents::{
        AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, new_result_slot,
    };

    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let state = test_state(&config, None);
    let subagent_id = "web-probe-run-2-implementer";
    BACKGROUND_AGENTS
        .register(BackgroundAgentHandle {
            agent_id: uuid::Uuid::new_v4(),
            subagent_id: subagent_id.to_string(),
            join_handle: None,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            spawned_at: std::time::SystemTime::now(),
            status: std::sync::Arc::new(std::sync::Mutex::new(AgentStatus::Running)),
            result_slot: new_result_slot(),
        })
        .expect("register pipeline agent");

    let snapshot = get_json(build_app(&config, state), "/api/agents/live").await;
    let entry = snapshot["agents"]
        .as_array()
        .expect("agents array")
        .iter()
        .find(|agent| agent["id"].as_str() == Some(subagent_id))
        .unwrap_or_else(|| panic!("pipeline agent missing from projection: {snapshot}"));
    assert_eq!(entry["kind"], "background");
    assert_eq!(entry["status"], "running");
    assert_eq!(entry["label"], subagent_id);

    BACKGROUND_AGENTS.mark_terminal(subagent_id, AgentStatus::Finished);
}

/// A `TaskCreate` agent is now in both registries, so the projection has to
/// choose. The task row wins: it has a description and a real creation time,
/// where the registry entry has only an id.
#[tokio::test]
async fn a_task_create_agent_is_listed_once_as_its_task() {
    use archon_tools::background_agents::{
        AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, new_result_slot,
    };
    use archon_tools::task_manager::{TASK_MANAGER, TaskStatus};

    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let state = test_state(&config, None);
    let subagent_id = uuid::Uuid::new_v4().to_string();
    let task_id = TASK_MANAGER.create_task("web dashboard dedupe probe");
    TASK_MANAGER.set_agent_id(&task_id, &subagent_id);
    TASK_MANAGER.set_status(&task_id, TaskStatus::Running);
    BACKGROUND_AGENTS
        .register(BackgroundAgentHandle {
            agent_id: uuid::Uuid::parse_str(&subagent_id).expect("uuid-shaped"),
            subagent_id: subagent_id.clone(),
            join_handle: None,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            spawned_at: std::time::SystemTime::now(),
            status: std::sync::Arc::new(std::sync::Mutex::new(AgentStatus::Running)),
            result_slot: new_result_slot(),
        })
        .expect("register task-create agent");

    let snapshot = get_json(build_app(&config, state), "/api/agents/live").await;
    let ids = agent_ids(&snapshot);
    assert!(ids.contains(&task_id), "task row missing: {snapshot}");
    assert!(
        !ids.contains(&subagent_id),
        "the same agent was listed twice: {snapshot}"
    );

    TASK_MANAGER.stop_task(&task_id).expect("stop probe task");
    BACKGROUND_AGENTS.mark_terminal(&subagent_id, AgentStatus::Finished);
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

/// The workbench renders ingested PDFs through PDF.js. Without a `script-src`
/// the renderer is one bug away from executing whatever a malicious document
/// puts in the DOM, so the header is asserted rather than assumed.
#[tokio::test]
async fn shell_document_is_served_with_a_script_constraining_csp() {
    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let app = build_app(&config, test_state(&config, None));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let csp = response
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .expect("workbench shell must carry a CSP")
        .to_str()
        .unwrap()
        .to_string();

    let directive = |name: &str| {
        csp.split(';')
            .map(str::trim)
            .find(|part| part.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("CSP has no {name} directive: {csp}"))
            .split_whitespace()
            .skip(1)
            .collect::<Vec<_>>()
    };

    // Exact, not `contains`: an added source is what a regression looks like.
    assert_eq!(directive("script-src"), ["'self'", "'wasm-unsafe-eval'"]);
    // The pdf.worker must come from this origin, never a CDN.
    assert_eq!(directive("worker-src"), ["'self'"]);
    assert_eq!(directive("object-src"), ["'none'"]);
    assert_eq!(directive("frame-ancestors"), ["'none'"]);
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
}

#[tokio::test]
async fn corpus_binary_bytes_reject_paths_outside_corpus_roots() {
    let config = WebConfig {
        open_browser: false,
        ..WebConfig::default()
    };
    let app = build_app(&config, test_state(&config, None));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/corpus/source/bytes?path=/etc/passwd")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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
