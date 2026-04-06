use serde_json::json;

use archon_tools::bash::BashTool;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext};

fn test_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-bash".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
    }
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_echo_hello() {
    let tool = BashTool::default();
    let result = tool
        .execute(json!({ "command": "echo hello" }), &test_ctx())
        .await;
    assert!(!result.is_error, "echo should succeed: {}", result.content);
    assert!(result.content.trim().contains("hello"));
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_exit_code_nonzero() {
    let tool = BashTool::default();
    let result = tool
        .execute(json!({ "command": "exit 1" }), &test_ctx())
        .await;
    assert!(result.is_error, "exit 1 should be error");
    assert!(result.content.contains("Exit code 1"));
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_timeout() {
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 102400,
    };
    let result = tool
        .execute(
            json!({ "command": "sleep 30", "timeout": 500 }),
            &test_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("timed out"),
        "should mention timeout: {}",
        result.content
    );
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_output_truncation() {
    let tool = BashTool {
        timeout_secs: 10,
        max_output_bytes: 100,
    };
    let result = tool
        .execute(
            // Generate output larger than 100 bytes
            json!({ "command": "seq 1 1000" }),
            &test_ctx(),
        )
        .await;
    assert!(
        result.content.contains("truncated"),
        "should mention truncation: {}",
        result.content
    );
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_sensitive_env_stripped() {
    let tool = BashTool::default();
    // Set a sensitive env var and check it's not visible
    // SAFETY: test-only, single-threaded test context
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-test-secret") };
    let result = tool
        .execute(json!({ "command": "echo $ANTHROPIC_API_KEY" }), &test_ctx())
        .await;
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };

    assert!(!result.is_error);
    assert!(
        !result.content.contains("sk-test-secret"),
        "API key should be stripped from env"
    );
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_working_directory() {
    // Canonicalize to resolve symlinks (e.g. macOS /var -> /private/var),
    // since `pwd` returns the physical path by default.
    let dir = std::fs::canonicalize(std::env::temp_dir()).expect("canonicalize temp dir");
    let ctx = ToolContext {
        working_dir: dir.clone(),
        session_id: "test".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
    };
    let tool = BashTool::default();
    let result = tool.execute(json!({ "command": "pwd" }), &ctx).await;
    assert!(!result.is_error);
    // pwd output should contain the temp dir path
    let expected = dir.to_string_lossy();
    assert!(
        result.content.contains(expected.as_ref()),
        "pwd should show working dir {expected}, got: {}",
        result.content
    );
}

#[test]
fn bash_permission_classification() {
    let tool = BashTool::default();
    assert_eq!(
        tool.permission_level(&json!({ "command": "ls" })),
        PermissionLevel::Safe
    );
    assert_eq!(
        tool.permission_level(&json!({ "command": "git commit -m 'x'" })),
        PermissionLevel::Risky
    );
    assert_eq!(
        tool.permission_level(&json!({ "command": "rm -rf /" })),
        PermissionLevel::Dangerous
    );
}

#[tokio::test]
async fn bash_missing_command() {
    let tool = BashTool::default();
    let result = tool.execute(json!({}), &test_ctx()).await;
    assert!(result.is_error);
    assert!(result.content.contains("command"));
}
