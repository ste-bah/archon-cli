//! Tests for the shared PTY session (#189 Phase 6).
//!
//! These spawn real shells through real ConPTY / `openpty`. That is the point:
//! the reason this crate exists is that a PTY behaves differently from a pipe,
//! so a test against a fake would confirm nothing about the thing being shared.

use super::*;

/// An interactive shell, reading commands from the PTY.
fn interactive_shell() -> CommandBuilder {
    if cfg!(windows) {
        CommandBuilder::new("cmd.exe")
    } else {
        CommandBuilder::new("/bin/sh")
    }
}

/// A command that runs long enough that it must be killed rather than awaited.
fn long_runner() -> CommandBuilder {
    let mut command = if cfg!(windows) {
        let mut command = CommandBuilder::new("cmd.exe");
        command.args(["/C", "ping -n 60 127.0.0.1"]);
        command
    } else {
        let mut command = CommandBuilder::new("/bin/sh");
        command.args(["-c", "sleep 60"]);
        command
    };
    command.cwd(std::env::temp_dir());
    command
}

fn small() -> PtySize {
    PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// A command whose *result* differs from its own echo.
///
/// A PTY echoes what is typed, so asserting on the literal command text would
/// pass even if the child never ran. Computing the value inside the shell means
/// the marker can only appear because the shell executed it.
/// Two lines on Windows rather than one: `cmd` expands `%MARK%` when it parses
/// the line, so a `set` and its use on the same line would print the literal
/// variable back.
fn marker_command() -> &'static str {
    if cfg!(windows) {
        "set /a MARK=6*7 >nul\r\necho m%MARK%k\r\n"
    } else {
        "echo m$((6*7))k\n"
    }
}

/// Drain output until `needle` shows up, or give up.
async fn read_until(session: &mut PtySession, needle: &str) -> String {
    let mut seen = String::new();
    let deadline = std::time::Duration::from_secs(20);
    let _ = tokio::time::timeout(deadline, async {
        while let Some(chunk) = session.next_output().await {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains(needle) {
                return;
            }
        }
    })
    .await;
    seen
}

#[tokio::test]
async fn input_reaches_the_child_and_its_output_comes_back() {
    let mut session = PtySession::spawn_headless(interactive_shell(), small()).expect("spawn");

    session.send_input(marker_command().as_bytes().to_vec());
    let seen = read_until(&mut session, "m42k").await;

    assert!(
        seen.contains("m42k"),
        "the shell never evaluated the command; saw: {seen:?}"
    );
}

/// The whole reason for a persistent session: state set by one command is still
/// there for the next one. A one-shot `Bash` call cannot do this.
#[tokio::test]
async fn state_survives_between_writes() {
    let mut session = PtySession::spawn_headless(interactive_shell(), small()).expect("spawn");

    let (set, show) = if cfg!(windows) {
        ("set KEPT=held\r\n", "echo [%KEPT%]\r\n")
    } else {
        ("KEPT=held\n", "echo \"[$KEPT]\"\n")
    };
    session.send_input(set.as_bytes().to_vec());
    // Wait for the first command to be consumed before sending the second, so
    // this tests persistence rather than the shell's input buffering.
    let _ = read_until(&mut session, "KEPT").await;
    session.send_input(show.as_bytes().to_vec());

    let seen = read_until(&mut session, "[held]").await;
    assert!(
        seen.contains("[held]"),
        "the variable did not survive the second write; saw: {seen:?}"
    );
}

#[tokio::test]
async fn the_child_pid_is_reported() {
    let session = PtySession::spawn_headless(long_runner(), small()).expect("spawn");
    assert!(session.child_pid().is_some(), "no pid for a live child");
}

/// Resizing a live PTY must not be an error path. The browser pane calls this
/// on every window drag.
#[tokio::test]
async fn a_live_session_can_be_resized() {
    let session = PtySession::spawn_headless(interactive_shell(), small()).expect("spawn");
    session.resize(40, 120);
    // A zero size is nonsense the caller can plausibly produce mid-teardown;
    // it is clamped rather than passed through to the kernel.
    session.resize(0, 0);
}

/// The guarantee the whole design rests on: nothing is left running.
///
/// Asserted through EOF rather than by asking the OS about the pid, because
/// EOF is what every consumer actually observes — and on Windows a pid can be
/// reused, so "the pid is gone" is the weaker claim of the two.
#[tokio::test]
async fn dropping_the_control_handle_kills_the_child() {
    let session = PtySession::spawn_headless(long_runner(), small()).expect("spawn");
    // Prove the child is alive first, so the EOF below means "killed" rather
    // than "never started".
    assert!(session.child_pid().is_some());
    let (control, mut output) = session.split();

    // No explicit `kill` — this is the `Drop` path, which is the one that has
    // to hold when a task is cancelled or unwinds. Dropping a whole
    // `PtySession` reaches the same code, since it owns this handle.
    drop(control);

    let ended = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while output.recv().await.is_some() {}
    })
    .await;

    assert!(
        ended.is_ok(),
        "output never ended, so the child outlived its session"
    );
}

/// An explicit `kill` stops the child, and the stream ends when the handle
/// goes — two events, not one.
///
/// Windows draws the distinction: the reader sees EOF when the ConPTY closes,
/// which is on drop, not when the child dies. Killing twice is the ordinary
/// case, since `Drop` runs after any explicit close, and must stay quiet.
#[tokio::test]
async fn an_explicit_kill_is_repeatable_and_the_stream_ends_on_drop() {
    let session = PtySession::spawn_headless(long_runner(), small()).expect("spawn");
    let (control, mut output) = session.split();
    assert!(control.child_pid().is_some());

    control.kill();
    control.kill();
    drop(control);

    let ended = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while output.recv().await.is_some() {}
    })
    .await;
    assert!(
        ended.is_ok(),
        "output never ended after the handle was gone"
    );
}

/// The headless reply is what makes a shell usable with no emulator attached.
/// Without it the first four bytes are the whole session — verified before this
/// existed, and the reason `spawn` and `spawn_headless` are separate calls.
#[tokio::test]
async fn a_headless_session_gets_past_the_terminal_handshake() {
    let mut session = PtySession::spawn_headless(interactive_shell(), small()).expect("spawn");

    session.send_input(marker_command().as_bytes().to_vec());
    let seen = read_until(&mut session, "m42k").await;

    assert!(
        seen.len() > 8,
        "the session produced only a handshake and stalled: {seen:?}"
    );
    assert!(seen.contains("m42k"), "saw: {seen:?}");
}
