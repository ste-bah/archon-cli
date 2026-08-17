use std::time::Duration;

use serde_json::json;

use super::bash_containment::{BashContainment, containment_for_platform};
use super::*;
use crate::tool::ToolContext;

#[test]
fn macos_uses_best_effort_process_group_cleanup_without_unshare() {
    assert!(matches!(
        containment_for_platform("macos"),
        BashContainment::ProcessGroup
    ));
}

#[test]
fn linux_uses_process_group_containment_without_unshare() {
    assert!(matches!(
        containment_for_platform("linux"),
        BashContainment::ProcessGroup
    ));
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn normal_completion_kills_setsid_escaped_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let started_file = dir.path().join("setsid-started");
    let escaped_file = dir.path().join("escaped-write");
    let tool = BashTool {
        max_output_bytes: 1024,
        ..Default::default()
    };

    let result = tool
        .execute(
            json!({
                "command": format!(
                    "setsid sh -c 'printf started > {}; sleep 0.2; printf escaped > {}' & while [ ! -s {} ]; do sleep 0.01; done",
                    shell_quote(&started_file),
                    shell_quote(&escaped_file),
                    shell_quote(&started_file),
                )
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(started_file.exists(), "setsid child never detached");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !escaped_file.exists(),
        "setsid descendant mutated the tree after Bash returned"
    );
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn inner_shell_exit_trap_cannot_disable_guard_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let started_file = dir.path().join("trap-setsid-started");
    let escaped_file = dir.path().join("trap-escaped-write");
    let tool = BashTool {
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tool
        .execute(
            json!({
                "command": format!(
                    "trap - EXIT; setsid sh -c 'printf started > {}; sleep 0.2; printf escaped > {}' & while [ ! -s {} ]; do sleep 0.01; done",
                    shell_quote(&started_file),
                    shell_quote(&escaped_file),
                    shell_quote(&started_file),
                )
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !escaped_file.exists(),
        "user trap replacement disabled descendant cleanup"
    );
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn inner_shell_cannot_kill_guard_via_parent_pid() {
    let dir = tempfile::tempdir().unwrap();
    let started_file = dir.path().join("parent-kill-started");
    let escaped_file = dir.path().join("parent-kill-escaped-write");
    let tool = BashTool {
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tool
        .execute(
            json!({
                "command": format!(
                    "setsid sh -c 'printf started > {}; sleep 0.2; printf escaped > {}' & while [ ! -s {} ]; do sleep 0.01; done; kill -KILL \"$PPID\"",
                    shell_quote(&started_file),
                    shell_quote(&escaped_file),
                    shell_quote(&started_file),
                )
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(started_file.exists(), "setsid child never detached");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !escaped_file.exists(),
        "inner shell killed its containment guard via $PPID"
    );
    assert!(
        result.content.contains("Exit code") || !result.is_error,
        "unexpected result: {}",
        result.content
    );
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn timeout_kills_setsid_escaped_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let started_file = dir.path().join("timeout-setsid-started");
    let escaped_file = dir.path().join("timeout-escaped-write");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tool
        .execute(
            json!({
                "command": format!(
                    "setsid sh -c 'printf started > {}; sleep 1.2; printf escaped > {}' & while [ ! -s {} ]; do sleep 0.01; done; sleep 30",
                    shell_quote(&started_file),
                    shell_quote(&escaped_file),
                    shell_quote(&started_file),
                ),
                "timeout": 100
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(result.is_error, "command should time out");
    assert!(started_file.exists(), "setsid child never detached");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !escaped_file.exists(),
        "setsid descendant mutated the tree after Bash timeout"
    );
}

fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\"'\"'"))
}
