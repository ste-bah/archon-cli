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
                "command": format!(
                    "sh -c 'trap \"\" TERM; while :; do sleep 1; done' & echo $! > {}; wait",
                    pid_file.display()
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

#[cfg(target_os = "macos")]
#[tokio::test]
async fn cargo_command_cannot_override_host_target_dir() {
    let working_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if !working_dir.starts_with("/Volumes/") {
        return;
    }
    let tool = BashTool::default();
    let result = tool
        .execute(
            json!({
                "command": "CARGO_TARGET_DIR=target/agent-owned cargo --version >/dev/null; printf '%s\\n%s\\n' \"$CARGO_TARGET_DIR\" \"$ARCHON_CARGO_TARGET_DIR\""
            }),
            &ToolContext {
                working_dir,
                session_id: "cargo-target-override-execution".to_string(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    let target_dirs: Vec<&str> = result.content.lines().collect();
    assert_eq!(target_dirs.len(), 2, "{}", result.content);
    assert_eq!(target_dirs[0], target_dirs[1]);
    assert!(!target_dirs[0].contains("agent-owned"));
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "slow D51 fresh-worktree build regression"]
async fn fresh_worktree_cargo_build_generates_tree_sitter_outputs() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf();
    if !repository.starts_with("/Volumes/") {
        return;
    }
    let worktree = repository
        .parent()
        .expect("workspace parent")
        .join(format!(".d51-worktree-{}", std::process::id()));
    assert!(
        !worktree.exists(),
        "stale test worktree: {}",
        worktree.display()
    );
    let add = std::process::Command::new("git")
        .current_dir(&repository)
        .args(["worktree", "add", "--detach"])
        .arg(&worktree)
        .arg("HEAD")
        .output()
        .expect("git worktree add");
    assert!(
        add.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    struct WorktreeCleanup {
        repository: PathBuf,
        worktree: PathBuf,
    }
    impl Drop for WorktreeCleanup {
        fn drop(&mut self) {
            let _ = std::process::Command::new("git")
                .current_dir(&self.repository)
                .args(["worktree", "remove", "--force"])
                .arg(&self.worktree)
                .status();
        }
    }
    let _cleanup = WorktreeCleanup {
        repository,
        worktree: worktree.clone(),
    };
    let tool = BashTool {
        timeout_secs: 1_200,
        max_output_bytes: 1_048_576,
        provider_env: None,
    };
    let result = tool
        .execute(
            json!({
                "command": "CARGO_TARGET_DIR=target/task-d51 cargo test -p archon-cli-workspace one_retry_generation_does_not_escalate -- --nocapture; status=$?; find \"$ARCHON_CARGO_TARGET_DIR/debug/build\" -path '*/tree-sitter-*/out/stdlib-symbols.txt' -print -quit; exit $status"
            }),
            &ToolContext {
                working_dir: worktree,
                session_id: "cargo-target-fresh-worktree".to_string(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(
        result
            .content
            .contains("one_retry_generation_does_not_escalate ... ok"),
        "focused test did not execute: {}",
        result.content
    );
    assert!(
        result.content.contains("stdlib-symbols.txt"),
        "tree-sitter build output was not generated: {}",
        result.content
    );
}

#[cfg(unix)]
fn process_exists(pid: &str) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
