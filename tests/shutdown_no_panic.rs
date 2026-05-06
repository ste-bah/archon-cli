//! Regression tests for Tokio shutdown panic fix.
//!
//! Proves the archon binary does NOT panic with
//! "A Tokio 1.x context was found, but it is being shutdown"
//! during clean exit.

use std::io;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

const CHILD_TIMEOUT: Duration = Duration::from_secs(15);

/// Locate the archon binary via the standard Cargo env var.
fn archon_bin() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_archon").map(PathBuf::from)
}

/// Helper: spawn archon with piped stdin, send input after a delay, capture stderr.
async fn run_archon_with_input(
    args: &[&str],
    stdin_bytes: &[u8],
    presleep_ms: u64,
) -> std::io::Result<std::process::Output> {
    let bin = archon_bin().expect("CARGO_BIN_EXE_archon not set");
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child.stdin.take().expect("stdin pipe");
    let stdin_data = stdin_bytes.to_vec();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(presleep_ms)).await;
        let _ = stdin.write_all(&stdin_data).await;
        let _ = stdin.shutdown().await;
    });

    let mut stdout = child.stdout.take().expect("stdout pipe");
    let mut stderr = child.stderr.take().expect("stderr pipe");
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).await.map(|_| buf)
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        stderr.read_to_end(&mut buf).await.map(|_| buf)
    });

    let status = match tokio::time::timeout(CHILD_TIMEOUT, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("archon did not exit within {CHILD_TIMEOUT:?}"),
            ));
        }
    };

    let stdout = stdout_task
        .await
        .map_err(|e| io::Error::other(format!("stdout reader task failed: {e}")))??;
    let stderr = stderr_task
        .await
        .map_err(|e| io::Error::other(format!("stderr reader task failed: {e}")))??;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

// ---------------------------------------------------------------------------
// Test 1: interactive mode clean shutdown — no runtime panic
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_mode_clean_shutdown_no_runtime_panic() {
    let output = run_archon_with_input(&[], b"/q\n", 2000)
        .await
        .expect("spawn archon");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // The binary may exit non-zero because raw mode fails on piped stdin.
    // That's expected — the test validates NO panic, not success exit code.
    assert!(
        !stderr.contains("A Tokio 1.x context was found, but it is being shutdown"),
        "shutdown panic in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked at") || !stderr.contains("runtime/time/entry.rs"),
        "runtime/time/entry.rs panic in stderr:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: print mode baseline — no runtime panic
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_mode_basic_smoke_no_panic() {
    let output = run_archon_with_input(&["-p", "echo hello"], b"", 0)
        .await
        .expect("spawn archon");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("A Tokio 1.x context was found, but it is being shutdown"),
        "shutdown panic in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked at") || !stderr.contains("runtime/time/entry.rs"),
        "runtime/time/entry.rs panic in stderr:\n{stderr}"
    );
}
