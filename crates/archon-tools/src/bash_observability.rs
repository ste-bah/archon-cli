use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use archon_observability::{AgentActivityEvent, AgentActivityKind, AgentActivityStatus};
use tokio::task::JoinHandle;

use crate::tool::ToolContext;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) fn start_bash_heartbeat(
    ctx: &ToolContext,
    pid: Option<u32>,
    timeout_ms: u64,
    command: &str,
    stdout_bytes: Arc<AtomicUsize>,
    stderr_bytes: Arc<AtomicUsize>,
) -> Option<JoinHandle<()>> {
    let sink = ctx.activity_sink.clone()?;
    let session_id = ctx.session_id.clone();
    let cwd = display_path(&ctx.working_dir);
    let label = command_label(command);
    let fingerprint = command_fingerprint(command);
    let run_id = session_id.starts_with("wf-").then(|| session_id.clone());

    emit(
        sink.as_ref(),
        &session_id,
        run_id.as_deref(),
        bash_message(pid, 0, timeout_ms, &cwd, label, fingerprint, 0, 0),
    );

    Some(tokio::spawn(async move {
        let mut elapsed = 0_u64;
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            interval.tick().await;
            elapsed += HEARTBEAT_INTERVAL.as_secs();
            emit(
                sink.as_ref(),
                &session_id,
                run_id.as_deref(),
                bash_message(
                    pid,
                    elapsed,
                    timeout_ms,
                    &cwd,
                    label,
                    fingerprint,
                    stdout_bytes.load(Ordering::Relaxed),
                    stderr_bytes.load(Ordering::Relaxed),
                ),
            );
        }
    }))
}

pub(crate) fn stop_bash_heartbeat(heartbeat: Option<JoinHandle<()>>) {
    if let Some(heartbeat) = heartbeat {
        heartbeat.abort();
    }
}

fn emit(
    sink: &dyn archon_observability::AgentActivitySink,
    session_id: &str,
    run_id: Option<&str>,
    message: String,
) {
    let mut event = AgentActivityEvent::new(
        session_id.to_string(),
        AgentActivityKind::ToolStarted,
        AgentActivityStatus::Running,
        message,
    );
    if let Some(run_id) = run_id {
        event = event.with_run_id(run_id.to_string());
    }
    sink.emit(event);
}

fn bash_message(
    pid: Option<u32>,
    elapsed_secs: u64,
    timeout_ms: u64,
    cwd: &str,
    label: &'static str,
    fingerprint: u64,
    stdout_bytes: usize,
    stderr_bytes: usize,
) -> String {
    format!(
        "Bash running pid={} elapsed={}s timeout={}s stdout={}B stderr={}B cwd={} command={} hash={:016x}",
        pid.map(|pid| pid.to_string()).unwrap_or_else(|| "?".into()),
        elapsed_secs,
        timeout_ms / 1000,
        stdout_bytes,
        stderr_bytes,
        cwd,
        label,
        fingerprint
    )
}

fn command_label(command: &str) -> &'static str {
    let lower = command.to_ascii_lowercase();
    if lower.contains("cargo nextest") {
        "cargo nextest"
    } else if lower.contains("cargo test") {
        "cargo test"
    } else if lower.contains("cargo build") {
        "cargo build"
    } else if lower.contains("cargo check") {
        "cargo check"
    } else if lower.contains("cargo clippy") {
        "cargo clippy"
    } else if lower.contains("cargo fmt") {
        "cargo fmt"
    } else if lower.contains("./gradlew") || lower.contains("gradle ") {
        "gradle"
    } else if lower.contains("npm test") || lower.contains("pnpm test") {
        "js test"
    } else if lower.contains("pytest") {
        "pytest"
    } else {
        "shell"
    }
}

fn command_fingerprint(command: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in command.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_label_uses_safe_categories() {
        assert_eq!(command_label("cd repo && cargo test foo"), "cargo test");
        assert_eq!(command_label("./gradlew :app:test"), "gradle");
        assert_eq!(
            command_label("curl https://example.test?token=secret"),
            "shell"
        );
    }

    #[test]
    fn fingerprint_is_stable_without_exposing_command() {
        let first = command_fingerprint("cargo test foo");
        let second = command_fingerprint("cargo test foo");
        assert_eq!(first, second);
        let rendered = bash_message(Some(42), 30, 86_400_000, ".", "cargo test", first, 1, 2);
        assert!(rendered.contains("pid=42"));
        assert!(rendered.contains("command=cargo test"));
        assert!(!rendered.contains("cargo test foo"));
    }
}
