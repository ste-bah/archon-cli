//! Which PostToolUse hook failures reach the tool result, and how they name
//! the hook there. Only no-progress stops are appended; every other
//! non-blocking error stays in the log. The model-visible line names the hook
//! by id, event and program, never by its argument values.
use super::tool_types::PreflightResult;
use super::*;
use crate::hooks::{HookCommandType, HookEvent, HookOutcome, HookResult, compute_hook_id};
use archon_tools::tool::{PermissionLevel, Tool, WorkingTreeEffect};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct ReadOnlyTool;
#[async_trait::async_trait]
impl Tool for ReadOnlyTool {
    fn name(&self) -> &str {
        "HookScope"
    }
    fn description(&self) -> &str {
        "read-only test tool"
    }
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn permission_level(&self, _: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
        ToolResult::success("read completed")
    }
}

/// Run one tool call through dispatch and postprocess with the given
/// PostToolUse session hooks, and return the published tool result text.
async fn post_tool_content(
    hooks: &[serde_json::Value],
    callback: Option<crate::hooks::HookCallbackEntry>,
) -> String {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(crate::hooks::HookRegistry::new());
    for hook in hooks {
        registry.register_session_hook(
            "scope",
            HookEvent::PostToolUse,
            serde_json::from_value(hook.clone()).unwrap(),
        );
    }
    if let Some(entry) = callback {
        registry.register_callback(HookEvent::PostToolUse, entry);
    }
    let mut agent = super::tests::test_agent();
    let (tx, mut events) = tokio::sync::mpsc::channel(32);
    agent.event_tx = tx;
    agent.config.working_dir = dir.path().to_path_buf();
    agent.config.session_id = "scope".into();
    agent.set_hook_registry(registry);
    let pre = PreflightResult {
        tool_name: "HookScope".into(),
        tool_id: "scope-1".into(),
        input: serde_json::json!({}),
        tool_arc: Arc::new(ReadOnlyTool),
        file_path: None,
        filesystem_effect: WorkingTreeEffect::None,
        filesystem_before: None,
        sandbox_prechecked: true,
    };
    let ctx = ToolContext::default();
    let results = agent
        .dispatch_allowed_tools(std::slice::from_ref(&pre), &ctx)
        .await;
    let stop = agent.postprocess_tools(&[pre], results, &ctx, "test").await;
    assert!(
        stop.is_none(),
        "an allowing hook result prevented continuation"
    );
    while let Ok(event) = events.try_recv() {
        if let AgentEvent::ToolCallComplete { result, .. } = event.inner {
            assert!(!result.is_error, "an allowing hook result failed the tool");
            return result.content;
        }
    }
    panic!("dispatch must publish tool completion");
}

fn command_hook(kind: &str, command: &str) -> serde_json::Value {
    serde_json::json!({"type": kind, "command": command, "timeout": 2, "on_failure": "allow"})
}

fn session_hook_id(kind: &HookCommandType, command: &str) -> String {
    compute_hook_id(&HookEvent::PostToolUse, kind, command, None)
}

/// A loopback HTTP server that answers every request with `response`, or
/// never answers when it is `None`. Returns the base URL.
async fn raw_http_server(response: Option<&'static str>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = [0_u8; 4096];
            let _ = socket.read(&mut request).await;
            match response {
                Some(text) => {
                    let _ = socket.write_all(text.as_bytes()).await;
                }
                None => held.push(socket),
            }
        }
    });
    format!("http://127.0.0.1:{port}")
}

// --- Only no-progress stops are appended (other errors stay in the log) ---

#[tokio::test]
async fn nonzero_exit_is_logged_not_appended() {
    let content = post_tool_content(&[command_hook("command", "exit 1")], None).await;
    assert_eq!(content, "read completed");
}

#[tokio::test]
async fn prompt_hook_nonzero_exit_is_logged_not_appended() {
    let content = post_tool_content(&[command_hook("prompt", "exit 3")], None).await;
    assert_eq!(content, "read completed");
}

#[tokio::test]
async fn http_status_error_is_logged_not_appended() {
    let base = raw_http_server(Some(
        "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    ))
    .await;
    let content = post_tool_content(&[command_hook("http", &format!("{base}/hook"))], None).await;
    assert_eq!(content, "read completed");
}

#[tokio::test]
async fn callback_nonblocking_result_is_not_appended() {
    let entry = crate::hooks::HookCallbackEntry {
        name: "explicit-error".into(),
        authority: crate::hooks::SourceAuthority::User,
        timeout_secs: 5,
        callback: Arc::new(|_| HookResult {
            outcome: HookOutcome::NonBlockingError,
            reason: Some("callback reported a soft error".into()),
            ..Default::default()
        }),
    };
    let content = post_tool_content(&[], Some(entry)).await;
    assert_eq!(content, "read completed");
}

// --- The stop line names the hook without its argument values ---

#[tokio::test]
#[tracing_test::traced_test]
async fn stop_warning_omits_argument_values() {
    let command = "sh -c 'exec sleep 600' sk-live-argv-7f3e";
    let content = post_tool_content(&[command_hook("command", command)], None).await;
    assert_eq!(
        content.matches("[Hook Warning] PostToolUse: ").count(),
        1,
        "{content}"
    );
    assert!(content.contains("no progress"), "{content}");
    let id = session_hook_id(&HookCommandType::Command, command);
    assert!(
        content.contains(&id),
        "warning lacks hook id {id}: {content}"
    );
    assert!(
        content.contains("`sh ...`"),
        "warning lacks program: {content}"
    );
    assert!(
        !content.contains("sk-live-argv-7f3e"),
        "argument leaked: {content}"
    );
    // The full command remains available to the operator in the log.
    assert!(
        logs_contain("sk-live-argv-7f3e"),
        "full command missing from log"
    );
}

#[tokio::test]
async fn stop_warning_omits_environment_assignments() {
    let command = "SECRET_TOKEN=sk-live-env-1a2b sleep 600";
    let content = post_tool_content(&[command_hook("command", command)], None).await;
    let id = session_hook_id(&HookCommandType::Command, command);
    assert!(
        content.contains(&id),
        "warning lacks hook id {id}: {content}"
    );
    assert!(content.contains("no progress"), "{content}");
    assert!(
        !content.contains("sk-live-env-1a2b"),
        "assignment leaked: {content}"
    );
    assert!(
        !content.contains("SECRET_TOKEN"),
        "assignment leaked: {content}"
    );
}

#[tokio::test]
async fn http_stop_warning_shows_only_the_origin() {
    let base = raw_http_server(None).await;
    let url = format!("{base}/hooks/sk-live-path-9c8d?token=sk-live-query-4e5f");
    let content = post_tool_content(&[command_hook("http", &url)], None).await;
    assert_eq!(
        content.matches("[Hook Warning] PostToolUse: ").count(),
        1,
        "{content}"
    );
    assert!(content.contains("no progress"), "{content}");
    let id = session_hook_id(&HookCommandType::Http, &url);
    assert!(
        content.contains(&id),
        "warning lacks hook id {id}: {content}"
    );
    assert!(content.contains(&base), "warning lacks origin: {content}");
    assert!(
        !content.contains("sk-live-path-9c8d"),
        "path leaked: {content}"
    );
    assert!(
        !content.contains("sk-live-query-4e5f"),
        "query leaked: {content}"
    );
}

// --- A callback stop follows the event's failure policy, never silently ---

#[tokio::test]
async fn post_tool_callback_stop_is_appended() {
    let entry = crate::hooks::HookCallbackEntry {
        name: "slow-callback".into(),
        authority: crate::hooks::SourceAuthority::User,
        timeout_secs: 1,
        callback: Arc::new(|_| {
            std::thread::sleep(std::time::Duration::from_millis(1_500));
            HookResult::allow()
        }),
    };
    let content = post_tool_content(&[], Some(entry)).await;
    assert_eq!(
        content.matches("[Hook Warning] PostToolUse: ").count(),
        1,
        "{content}"
    );
    assert!(content.contains("callback 'slow-callback'"), "{content}");
    assert!(content.contains("no progress"), "{content}");
}
