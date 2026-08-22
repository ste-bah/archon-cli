//! The terminal container: the one most likely to outlive the process that
//! opened it, and the one that had no bound at all.
//!
//! A `docker run -it` container does not stop when the client that attached to
//! it dies, so a SIGKILLed Archon left an interactive shell's container running.
//! It carried no labels either, so nothing could find it and reaping could never
//! collect it. Both halves are proved here against a real daemon.

use super::*;
use archon_permissions::sandbox::{SandboxTerminal, SandboxTerminalRequest};

/// Slow on purpose. `container_max_age_secs` has a 60s floor — a shorter bound
/// would kill commands mid-flight — so proving the bound *fires* costs a little
/// over a minute. Left in rather than trusted to the argument-level unit test,
/// because the argument being right and the container actually stopping are
/// different claims, and only one of them is the guarantee.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_terminal_container_is_labelled_and_stops_at_its_age_bound() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = DockerConfig {
        container_max_age_secs: 60,
        ..docker_config()
    };
    let backend = DockerSandboxBackend::new(config, "rw", SandboxScope::Session);

    let SandboxTerminal::Open(command) = backend.terminal(&SandboxTerminalRequest {
        shell: None,
        workspace: dir.path().to_path_buf(),
        cwd: dir.path().to_path_buf(),
    }) else {
        panic!("the docker backend must open a terminal in the container");
    };

    let mut builder = archon_pty::CommandBuilder::new(&command.program);
    builder.args(&command.args);
    let session = archon_pty::PtySession::spawn_headless(
        builder,
        archon_pty::PtySize {
            rows: 50,
            cols: 240,
            pixel_width: 0,
            pixel_height: 0,
        },
    )
    .expect("the docker terminal spawns");
    let (control, mut output) = session.split();

    // The shell has to be *up* before the container can be looked for, and the
    // marker is what says so. Waiting on the expanded text, never on anything
    // the typed line contains: the PTY echoes what was typed.
    control.send_input(b"printf 'SHELL_%s\\n' READY\n".to_vec());
    let mut seen = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline && !seen.contains("SHELL_READY") {
        match tokio::time::timeout(std::time::Duration::from_secs(5), output.recv()).await {
            Ok(Some(chunk)) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(None) => break,
            Err(_) => {}
        }
    }
    assert!(
        seen.contains("SHELL_READY"),
        "the terminal never came up: {seen}"
    );

    let id = terminal_container_id(dir.path())
        .expect("a terminal container mounted on this workspace, findable by label");
    let _cleanup = Removed(id.clone());

    // The owner is deliberately not killed: the point is that the container ends
    // on its own account, from inside, with nothing host-side helping it. That
    // is what has to hold when Archon is SIGKILLed and never restarted.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(100);
    while std::time::Instant::now() < deadline && container_exists(&id) {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    let survived = container_exists(&id);
    control.kill();

    assert!(
        !survived,
        "the terminal container outlived its age bound; a SIGKILLed owner would \
         leave this shell running forever"
    );
}

/// The id of the terminal container bound to `workspace`, by label.
///
/// Failing to find one *is* the F5 failure: an unlabelled container is one no
/// operator command and no reaping pass can ever see.
fn terminal_container_id(workspace: &Path) -> Option<String> {
    let listed = std::process::Command::new("docker")
        .args([
            "ps",
            "--quiet",
            "--no-trunc",
            "--filter",
            "label=archon.sandbox=1",
            "--filter",
            "label=archon.sandbox.kind=terminal",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .find(|id| {
            mount_sources(id).iter().any(|source| {
                std::path::Path::new(source) == workspace
                    || std::fs::canonicalize(source).ok().as_deref()
                        == std::fs::canonicalize(workspace).ok().as_deref()
            })
        })
}
