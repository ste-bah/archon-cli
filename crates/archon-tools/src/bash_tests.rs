use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncRead, ReadBuf};

use super::bash_output::spawn_counted_pipe_capture;

use serde_json::json;

use super::*;
use crate::provider_env::{ProviderEnvPolicy, ProviderEnvSource};
use crate::tool::{PermissionLevel, ToolContext};

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("."),
        ..ToolContext::default()
    }
}

/// Generous relative to process spawn, not to the work being timed.
///
/// The two tests below assert on `printf` output and shell out to a real
/// `bash` to get it. Spawning one costs a few hundred milliseconds cold on
/// Windows, and at the default test parallelism a dozen start at once, so one
/// second is not a margin: the suite failed on Windows at full width while
/// passing under `--test-threads=4`, which is a false red in a crate that is
/// otherwise clean. A genuinely hung shell still fails, just later. See #131.
const SHELL_SPAWN_TIMEOUT_SECS: u64 = 15;

#[tokio::test]
async fn printf_format_starting_with_dash_succeeds() {
    let tool = BashTool {
        timeout_secs: SHELL_SPAWN_TIMEOUT_SECS,
        max_output_bytes: 1024,
        ..Default::default()
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
        timeout_secs: SHELL_SPAWN_TIMEOUT_SECS,
        max_output_bytes: 1024,
        ..Default::default()
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

#[test]
fn provider_env_source_preserves_bash_configuration() {
    let tool = BashTool {
        timeout_secs: 17,
        max_output_bytes: 23,
        safe_commands: vec!["echo safe".to_string()],
        risky_commands: vec!["echo risky".to_string()],
        dangerous_commands: vec!["echo dangerous".to_string()],
        provider_env: None,
    }
    .with_provider_env_source(ProviderEnvSource::Policy(ProviderEnvPolicy::new(vec![
        "ARCHON_TEST_PROVIDER_KEY".to_string(),
    ])));

    assert_eq!(tool.timeout_secs, 17);
    assert_eq!(tool.max_output_bytes, 23);
    assert_eq!(
        tool.permission_level(&json!({"command": "echo safe value"})),
        PermissionLevel::Safe
    );
    assert_eq!(
        tool.permission_level(&json!({"command": "echo risky value"})),
        PermissionLevel::Risky
    );
    assert_eq!(
        tool.permission_level(&json!({"command": "echo dangerous value"})),
        PermissionLevel::Dangerous
    );
    assert!(tool.provider_env.is_some());
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
        ..Default::default()
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

#[test]
fn bash_program_selection_prefers_path_discovery() {
    let bash = PathBuf::from("/usr/local/bin/bash");
    let bash_exe = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    assert_eq!(
        select_bash_program(Some(bash.clone()), Some(bash_exe.clone())),
        bash
    );
    assert_eq!(select_bash_program(None, Some(bash_exe.clone())), bash_exe);
    assert_eq!(select_bash_program(None, None), PathBuf::from("bash"));
}

#[tokio::test]
async fn pipe_reader_caps_storage_and_drains_remaining_bytes() {
    let (mut writer, reader) = tokio::io::duplex(64);
    let byte_count = Arc::new(AtomicUsize::new(0));
    let task = spawn_counted_pipe_capture(
        Some(reader),
        Arc::new(AtomicUsize::new(5)),
        Arc::clone(&byte_count),
    );
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        writer.write_all(b"abcdefghij").await.unwrap();
    })
    .await
    .unwrap();

    let captured = task.await.unwrap();
    assert_eq!(captured.bytes, b"abcde");
    assert!(captured.truncated);
    assert_eq!(byte_count.load(Ordering::Relaxed), 10);
}

#[tokio::test]
async fn pipe_read_failure_is_preserved() {
    let captured = spawn_counted_pipe_capture(
        Some(FailingReader),
        Arc::new(AtomicUsize::new(5)),
        Arc::new(AtomicUsize::new(0)),
    )
    .await
    .unwrap();

    assert_eq!(captured.read_error.as_deref(), Some("fixture read failure"));
}

struct FailingReader;

impl AsyncRead for FailingReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("fixture read failure")))
    }
}

#[tokio::test]
async fn pipe_readers_share_one_total_capture_budget() {
    let remaining = Arc::new(AtomicUsize::new(6));
    let (mut stdout_writer, stdout_reader) = tokio::io::duplex(64);
    let (mut stderr_writer, stderr_reader) = tokio::io::duplex(64);
    let stdout_task = spawn_counted_pipe_capture(
        Some(stdout_reader),
        Arc::clone(&remaining),
        Arc::new(AtomicUsize::new(0)),
    );
    let stderr_task = spawn_counted_pipe_capture(
        Some(stderr_reader),
        remaining,
        Arc::new(AtomicUsize::new(0)),
    );

    let stdout_write = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        stdout_writer.write_all(b"stdout").await.unwrap();
    });
    let stderr_write = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        stderr_writer.write_all(b"stderr").await.unwrap();
    });
    stdout_write.await.unwrap();
    stderr_write.await.unwrap();

    let (stdout, stderr) = tokio::join!(stdout_task, stderr_task);
    let stdout = stdout.unwrap();
    let stderr = stderr.unwrap();
    assert_eq!(stdout.bytes.len() + stderr.bytes.len(), 6);
    assert!(stdout.truncated || stderr.truncated);
}
