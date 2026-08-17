use serde_json::json;

use archon_tools::bash::BashTool;
// Used only by `cfg(not(target_os = "windows"))` tests below. See #136.
#[cfg(not(target_os = "windows"))]
use archon_tools::provider_env::{ProviderEnvPolicy, resolve_provider_env};
use archon_tools::tool::{PermissionLevel, Tool, ToolContext};

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_result_carries_private_authoritative_execution_metadata() {
    let result = BashTool::default()
        .execute(
            json!({ "command": "printf 'authoritative'; exit 7" }),
            &test_ctx(),
        )
        .await;
    let execution = result
        .authoritative_bash_execution()
        .expect("real Bash execution metadata");

    assert_eq!(execution.command(), "printf 'authoritative'; exit 7");
    assert_eq!(execution.exit_code(), 7);
    assert_eq!(execution.output(), result.content);
}

fn test_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-bash".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

/// Both bounds, because they are only meaningful as a pair: a floor above the
/// ceiling would silently collapse to the ceiling and make the model's `timeout`
/// argument a no-op everywhere.
#[test]
fn bash_default_timeout_bounds_are_one_hour_and_thirty_minutes() {
    assert_eq!(BashTool::default().timeout_secs, 3600);
    assert_eq!(BashTool::default().timeout_floor_secs, 1800);
    assert!(BashTool::default().timeout_floor_secs < BashTool::default().timeout_secs);
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
async fn resolved_provider_env_is_stable_across_multiple_subagent_shells() {
    let temp = tempfile::tempdir().expect("tempdir");
    let profile = temp.path().join("profile");
    std::fs::write(
        &profile,
        "export ARCHON_TEST_PROVIDER_SNAPSHOT=first-value\n",
    )
    .expect("profile");
    let policy = ProviderEnvPolicy {
        required_keys: vec!["ARCHON_TEST_PROVIDER_SNAPSHOT".to_string()],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("D47 regression".to_string()),
    };
    let resolution = resolve_provider_env(&policy).await;
    std::fs::write(&profile, "unset ARCHON_TEST_PROVIDER_SNAPSHOT\n").expect("rewrite profile");
    let tool = BashTool::default().with_provider_env_resolution(resolution);

    for _ in 0..2 {
        let result = tool
            .execute(
                json!({
                    "command": "test -n \"${ARCHON_TEST_PROVIDER_SNAPSHOT:-}\" && echo present"
                }),
                &test_ctx(),
            )
            .await;
        assert!(!result.is_error, "snapshot missing: {}", result.content);
        assert_eq!(result.content.trim(), "present");
        assert!(!result.content.contains("first-value"));
    }
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
        ..Default::default()
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
async fn bash_clamps_longer_requested_timeout_to_configured_maximum() {
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 102400,
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let result = tool
        .execute(
            json!({ "command": "sleep 2", "timeout": 5_000 }),
            &test_ctx(),
        )
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("timed out"));
    assert!(start.elapsed() < std::time::Duration::from_millis(1_500));
}

/// A requested timeout BELOW the floor is raised to it, not honoured.
///
/// This test previously asserted the opposite — that `timeout: 50` cut a
/// `sleep 1` short — which was the behaviour before `tools.bash_timeout_floor`
/// existed. The floor was added precisely because a model-chosen timeout was
/// killing real builds: an agent would ask for 120s, a cold cargo compile
/// needed longer, and the task was recorded as failing on a timeout it had
/// selected for itself. Config is a floor now, not merely a ceiling, so the
/// short request loses and `sleep 1` runs to completion.
///
/// Renamed rather than deleted: the scenario still matters, only the expected
/// outcome inverted.
#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_raises_a_requested_timeout_below_the_floor() {
    let tool = BashTool {
        timeout_secs: 2,
        // Floor between the requested 50ms and the 2s ceiling, so the clamp is
        // what decides the outcome rather than either bound alone.
        timeout_floor_secs: 2,
        max_output_bytes: 102400,
        ..Default::default()
    };
    let result = tool
        .execute(json!({ "command": "sleep 1", "timeout": 50 }), &test_ctx())
        .await;

    assert!(
        !result.is_error,
        "a 50ms request must be raised to the floor, letting `sleep 1` finish: {}",
        result.content
    );
    assert!(
        !result.content.contains("timed out"),
        "the command completed, so nothing timed out: {}",
        result.content
    );
}

/// The ceiling still binds: a request ABOVE it is clamped down, so the floor
/// change did not turn the configured maximum into a suggestion.
#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_still_clamps_a_requested_timeout_above_the_ceiling() {
    let tool = BashTool {
        timeout_secs: 1,
        timeout_floor_secs: 1,
        max_output_bytes: 102400,
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let result = tool
        .execute(
            json!({ "command": "sleep 30", "timeout": 600_000 }),
            &test_ctx(),
        )
        .await;

    assert!(result.is_error, "the ceiling must still terminate it");
    assert!(result.content.contains("timed out"));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(10),
        "it must stop at the 1s ceiling, not run the full sleep"
    );
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn workflow_bash_ignores_shell_timeout_wrapper() {
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 102400,
        ..Default::default()
    };
    let ctx = ToolContext {
        session_id: "wf-test-run".into(),
        ..test_ctx()
    };
    let result = tool
        .execute(
            json!({ "command": "timeout 1ms sh -c 'sleep 0.05; echo shell-timeout-ignored'" }),
            &ctx,
        )
        .await;
    assert!(
        !result.is_error,
        "workflow Bash should ignore shell timeout wrappers and rely on configured timeout: {}",
        result.content
    );
    assert!(result.content.contains("shell-timeout-ignored"));
}

#[cfg(not(target_os = "windows"))]
#[tokio::test]
async fn bash_output_truncation() {
    let tool = BashTool {
        timeout_secs: 10,
        max_output_bytes: 100,
        ..Default::default()
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
        ..Default::default()
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

#[test]
fn bash_permission_classification_uses_configured_lists() {
    let tool = BashTool {
        safe_commands: vec!["cargo build".to_string()],
        risky_commands: vec!["ls".to_string()],
        dangerous_commands: vec!["echo deploy".to_string()],
        ..Default::default()
    };

    assert_eq!(
        tool.permission_level(&json!({ "command": "cargo build --release" })),
        PermissionLevel::Safe
    );
    assert_eq!(
        tool.permission_level(&json!({ "command": "ls -la" })),
        PermissionLevel::Risky
    );
    assert_eq!(
        tool.permission_level(&json!({ "command": "echo deploy production" })),
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
