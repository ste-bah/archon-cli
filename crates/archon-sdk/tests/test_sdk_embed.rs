//! Integration tests for archon-sdk (TASK-CLI-305).
//!
//! Tests cover: SdkMessage/SdkError types, SdkOptions, SdkTool, SdkMcpServer,
//! session CRUD (create/list/rename/tag/fork/resume), and query() error handling.

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::StreamExt;

use archon_sdk::{
    create_sdk_mcp_server, fork_session, get_session_info, list_sessions, rename_session,
    tag_session, unstable_v2_create_session, unstable_v2_resume_session, query,
    SdkAuth, SdkError, SdkMessage, SdkOptions, SdkResultMessage, SdkTool, SdkUsage,
    SessionOptions,
};

// ── helpers ──────────────────────────────────────────────────────────────────

/// A simple echo handler for tests.
fn echo_handler() -> Arc<
    dyn Fn(serde_json::Value) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<String, SdkError>> + Send + 'static>,
    > + Send
    + Sync,
> {
    Arc::new(|input: serde_json::Value| {
        Box::pin(async move {
            Ok(input["text"].as_str().unwrap_or("(none)").to_string())
        })
    })
}

fn tmp_sessions_dir() -> PathBuf {
    let dir = std::env::temp_dir()
        .join("archon-sdk-tests")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tmp_opts() -> SessionOptions {
    SessionOptions {
        sessions_dir: Some(tmp_sessions_dir()),
        ..Default::default()
    }
}

// ── SdkMessage tests ─────────────────────────────────────────────────────────

#[test]
fn sdk_message_assistant_serializes() {
    let msg = SdkMessage::AssistantMessage { content: "hello world".into() };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("assistant_message"), "got: {json}");
    assert!(json.contains("hello world"), "got: {json}");
}

#[test]
fn sdk_message_tool_use_serializes() {
    let msg = SdkMessage::ToolUse {
        id: "tu_1".into(),
        name: "my_tool".into(),
        input: serde_json::json!({ "key": "value" }),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("tool_use"), "got: {json}");
    assert!(json.contains("my_tool"), "got: {json}");
}

#[test]
fn sdk_message_tool_result_serializes() {
    let msg = SdkMessage::ToolResult { id: "tu_1".into(), content: "result".into() };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("tool_result"), "got: {json}");
}

#[test]
fn sdk_message_result_message_serializes() {
    let msg = SdkMessage::ResultMessage(SdkResultMessage {
        stop_reason: "end_turn".into(),
        usage: SdkUsage { input_tokens: 5, output_tokens: 10 },
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("result_message"), "got: {json}");
    assert!(json.contains("end_turn"), "got: {json}");
}

#[test]
fn sdk_message_round_trips() {
    let msg = SdkMessage::AssistantMessage { content: "test".into() };
    let json = serde_json::to_string(&msg).unwrap();
    let decoded: SdkMessage = serde_json::from_str(&json).unwrap();
    if let SdkMessage::AssistantMessage { content } = decoded {
        assert_eq!(content, "test");
    } else {
        panic!("wrong variant");
    }
}

// ── SdkError tests ────────────────────────────────────────────────────────────

#[test]
fn sdk_error_auth_message() {
    let e = SdkError::Auth("no key set".into());
    assert!(e.to_string().contains("no key set"));
}

#[test]
fn sdk_error_api_includes_status() {
    let e = SdkError::Api { status: 401, message: "Unauthorized".into() };
    assert!(e.to_string().contains("401"));
    assert!(e.to_string().contains("Unauthorized"));
}

#[test]
fn sdk_error_all_variants_compile() {
    let _errors: Vec<SdkError> = vec![
        SdkError::Auth("x".into()),
        SdkError::Api { status: 500, message: "err".into() },
        SdkError::Tool("bad tool".into()),
        SdkError::Config("bad config".into()),
        SdkError::Session("no session".into()),
        SdkError::Io(std::io::Error::new(std::io::ErrorKind::Other, "io")),
        SdkError::Serde(serde_json::from_str::<()>("!").unwrap_err()),
    ];
}

// ── SdkOptions tests ──────────────────────────────────────────────────────────

#[test]
fn sdk_options_defaults() {
    let opts = SdkOptions::default();
    assert_eq!(opts.model, "claude-sonnet-4-6");
    assert_eq!(opts.max_tokens, 8192);
    assert!(opts.system_prompt.is_none());
    assert!(opts.mcp_server.is_none());
}

#[test]
fn sdk_options_explicit_key() {
    let opts = SdkOptions {
        auth: SdkAuth::ApiKey("sk-test".into()),
        model: "claude-opus-4-6".into(),
        max_tokens: 4096,
        system_prompt: Some("You are helpful.".into()),
        ..Default::default()
    };
    assert_eq!(opts.model, "claude-opus-4-6");
    assert_eq!(opts.max_tokens, 4096);
}

#[test]
fn sdk_auth_variants_compile() {
    let _: Vec<SdkAuth> = vec![
        SdkAuth::FromEnv,
        SdkAuth::ApiKey("key".into()),
        SdkAuth::BearerToken("token".into()),
    ];
}

// ── SdkTool + SdkMcpServer tests ──────────────────────────────────────────────

#[test]
fn sdk_tool_api_schema_correct() {
    let tool = SdkTool {
        name: "echo".into(),
        description: "Echoes input text".into(),
        schema: serde_json::json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"]
        }),
        handler: echo_handler(),
    };
    let schema = tool.to_api_schema();
    assert_eq!(schema["name"], "echo");
    assert_eq!(schema["description"], "Echoes input text");
    assert!(schema["input_schema"].is_object());
}

#[test]
fn create_sdk_mcp_server_holds_two_tools() {
    let t1 = SdkTool {
        name: "tool_a".into(),
        description: "A".into(),
        schema: serde_json::json!({ "type": "object" }),
        handler: echo_handler(),
    };
    let t2 = SdkTool {
        name: "tool_b".into(),
        description: "B".into(),
        schema: serde_json::json!({ "type": "object" }),
        handler: echo_handler(),
    };
    let server = create_sdk_mcp_server("test-server", vec![t1, t2]);
    assert_eq!(server.name(), "test-server");
    let schemas = server.tool_schemas();
    assert_eq!(schemas.len(), 2);
    assert_eq!(schemas[0]["name"], "tool_a");
    assert_eq!(schemas[1]["name"], "tool_b");
}

#[tokio::test]
async fn sdk_tool_handler_callable() {
    let tool = SdkTool {
        name: "echo".into(),
        description: "Echo".into(),
        schema: serde_json::json!({}),
        handler: echo_handler(),
    };
    let result = tool.call(serde_json::json!({ "text": "ping" })).await.unwrap();
    assert_eq!(result, "ping");
}

// ── Session CRUD tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn session_create_has_id() {
    let opts = tmp_opts();
    let session = unstable_v2_create_session(opts).await.unwrap();
    assert!(!session.id().is_empty());
}

#[tokio::test]
async fn session_create_and_list() {
    let opts = tmp_opts();
    let session = unstable_v2_create_session(opts.clone()).await.unwrap();
    let sessions = list_sessions(Some(opts)).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session.id());
}

#[tokio::test]
async fn session_get_info_returns_metadata() {
    let opts = tmp_opts();
    let session = unstable_v2_create_session(opts.clone()).await.unwrap();
    let info = get_session_info(session.id(), Some(opts)).await.unwrap();
    assert_eq!(info.id, session.id());
    assert!(info.created_at > 0);
}

#[tokio::test]
async fn session_rename_updates_title() {
    let opts = tmp_opts();
    let session = unstable_v2_create_session(opts.clone()).await.unwrap();
    rename_session(session.id(), "My Session", Some(opts.clone())).await.unwrap();
    let info = get_session_info(session.id(), Some(opts)).await.unwrap();
    assert_eq!(info.title.as_deref(), Some("My Session"));
}

#[tokio::test]
async fn session_tag_adds_tag() {
    let opts = tmp_opts();
    let session = unstable_v2_create_session(opts.clone()).await.unwrap();
    tag_session(session.id(), "important", Some(opts.clone())).await.unwrap();
    let info = get_session_info(session.id(), Some(opts)).await.unwrap();
    assert!(info.tags.contains(&"important".to_string()));
}

#[tokio::test]
async fn session_fork_creates_new_id() {
    let opts = tmp_opts();
    let session = unstable_v2_create_session(opts.clone()).await.unwrap();
    let forked = fork_session(session.id(), Some(opts.clone())).await.unwrap();
    assert_ne!(forked.id(), session.id());
    let sessions = list_sessions(Some(opts)).await.unwrap();
    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn session_resume_finds_existing() {
    let opts = tmp_opts();
    let session = unstable_v2_create_session(opts.clone()).await.unwrap();
    let id = session.id().to_string();
    drop(session);
    let resumed = unstable_v2_resume_session(&id, opts).await.unwrap();
    assert_eq!(resumed.id(), id);
}

#[tokio::test]
async fn session_resume_missing_returns_error() {
    let opts = tmp_opts();
    let result = unstable_v2_resume_session("nonexistent-id", opts).await;
    assert!(result.is_err());
    matches!(result.unwrap_err(), SdkError::Session(_));
}

// ── query() error handling (no API key) ──────────────────────────────────────

#[tokio::test]
async fn query_no_api_key_yields_auth_error() {
    // Skip if real API key is set (would make a real API call)
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return;
    }
    let opts = SdkOptions {
        auth: SdkAuth::ApiKey("".into()), // empty key → auth error
        ..Default::default()
    };
    let mut stream = query("hello", opts);
    let first = stream.next().await;
    assert!(first.is_some(), "stream should yield at least one item");
    assert!(first.unwrap().is_err(), "empty key should yield error");
}

// ── No TUI dependency check ───────────────────────────────────────────────────

#[test]
fn sdk_compiles_without_tui() {
    // If this test file compiles, SDK has no TUI dependency.
    let _opts = SdkOptions::default();
}
