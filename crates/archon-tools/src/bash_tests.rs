use std::path::PathBuf;
use std::time::Duration;

use serde_json::json;

use super::*;
use crate::provider_env::ProviderEnvPolicy;
use crate::tool::ToolContext;

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("."),
        ..ToolContext::default()
    }
}

#[tokio::test]
async fn printf_format_starting_with_dash_succeeds() {
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        provider_env: None,
    };

    let result = tool
        .execute(json!({"command": "printf '--- heading ---\\n'"}), &ctx())
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "--- heading ---\n");
}

#[tokio::test]
async fn printf_wrapper_preserves_dash_dash_and_v() {
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        provider_env: None,
    };

    let result = tool
        .execute(
            json!({"command": "printf -- '--- one ---\\n'; printf -v label 'two'; printf '%s\\n' \"$label\""}),
            &ctx(),
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "--- one ---\ntwo\n");
}

#[tokio::test]
#[cfg(unix)]
async fn timeout_kills_background_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        provider_env: None,
    };
    let result = tool
        .execute(
            json!({
                "command": format!("sleep 30 & echo $! > {}; wait", pid_file.display()),
                "timeout": 100
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(result.is_error, "command should time out");
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    for _ in 0..20 {
        if !process_exists(&pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(&pid)
        .status();
    panic!("background sleep process survived Bash timeout: pid={pid}");
}

#[tokio::test]
async fn provider_env_overlay_is_scoped_and_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile");
    let secret = "provider-secret-value-123";
    std::fs::write(
        &profile,
        format!("export ARCHON_TEST_PROVIDER_SCOPED_KEY={secret}\n"),
    )
    .unwrap();
    let policy = ProviderEnvPolicy {
        required_keys: vec!["ARCHON_TEST_PROVIDER_SCOPED_KEY".to_string()],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("test".to_string()),
    };
    let tool = BashTool::default().with_provider_env(policy);

    let result = tool
        .execute(
            json!({"command": "printf '%s' \"$ARCHON_TEST_PROVIDER_SCOPED_KEY\""}),
            &ctx(),
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, "<redacted:ARCHON_TEST_PROVIDER_SCOPED_KEY>");
    assert!(!result.content.contains(secret));

    let ordinary = BashTool::default()
        .execute(
            json!({"command": "printf '%s' \"$ARCHON_TEST_PROVIDER_SCOPED_KEY\""}),
            &ctx(),
        )
        .await;
    assert!(!ordinary.is_error, "{}", ordinary.content);
    assert_eq!(ordinary.content, "");
}

#[cfg(unix)]
fn process_exists(pid: &str) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
