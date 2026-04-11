//! TASK-AGS-102: D3 AgentEvent channel flipped to unbounded (REQ-FOR-D3 [1/4]).
//!
//! Source-level regression guards. These tests enforce
//! SPEC-AGS-ARCH-FIXES US-ARCH-03/AC-01 + AC-02 and act as the TC-ARCH-05
//! precursor: grep the touched files and assert zero bounded sites remain
//! AND at least one unbounded site exists at each expected location. Written
//! BEFORE the flip (Gate 1).

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR for this integration test is the workspace bin
    // crate root (`archon-cli-workspace`), which is the repo root itself.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    let p: PathBuf = repo_root().join(path);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn no_bounded_agent_event_channel_in_main_rs() {
    let src = read("src/main.rs");
    assert_eq!(
        count_occurrences(&src, "mpsc::channel::<AgentEvent>"),
        0,
        "src/main.rs must not create a bounded AgentEvent channel (D3)"
    );
    assert_eq!(
        count_occurrences(&src, "mpsc::channel::<archon_core::agent::AgentEvent>"),
        0,
        "src/main.rs must not create a bounded fully-qualified AgentEvent channel"
    );
}

#[test]
fn no_bounded_agent_event_channel_in_agent_rs() {
    let src = read("crates/archon-core/src/agent.rs");
    assert!(
        !src.contains("mpsc::Sender<AgentEvent>"),
        "agent.rs must use UnboundedSender<AgentEvent>"
    );
}

#[test]
fn no_bounded_agent_event_channel_in_orchestrator_rs() {
    let src = read("crates/archon-core/src/orchestrator.rs");
    assert_eq!(
        count_occurrences(&src, "mpsc::channel::<AgentEvent>"),
        0,
        "orchestrator.rs must not create a bounded AgentEvent channel"
    );
}

#[test]
fn no_awaited_agent_event_send_in_agent_rs() {
    let src = read("crates/archon-core/src/agent.rs");
    assert_eq!(
        count_occurrences(&src, "self.event_tx.send(event).await"),
        0,
        "agent.rs must not .await on the AgentEvent sender (unbounded send is sync)"
    );
}

#[test]
fn main_rs_has_unbounded_agent_event_channels() {
    let src = read("src/main.rs");
    assert!(
        src.contains("unbounded_channel::<AgentEvent>")
            || src.contains("unbounded_channel::<archon_core::agent::AgentEvent>"),
        "src/main.rs must declare at least one unbounded AgentEvent channel"
    );
}

#[test]
fn agent_rs_uses_unbounded_sender_type() {
    let src = read("crates/archon-core/src/agent.rs");
    assert!(
        src.contains("UnboundedSender<AgentEvent>"),
        "agent.rs must use mpsc::UnboundedSender<AgentEvent>"
    );
}

#[test]
fn print_mode_receives_unbounded_receiver() {
    let src = read("crates/archon-core/src/print_mode.rs");
    assert!(
        src.contains("UnboundedReceiver<AgentEvent>"),
        "print_mode.rs signature must take UnboundedReceiver<AgentEvent>"
    );
    assert!(
        !src.contains("mpsc::Receiver<AgentEvent>"),
        "print_mode.rs must not take bounded Receiver<AgentEvent>"
    );
}

#[test]
fn sdk_ide_stdio_receives_unbounded_receiver() {
    let path = Path::new("crates/archon-sdk/src/ide/stdio.rs");
    let abs = repo_root().join(path);
    if !abs.exists() {
        // File may have been relocated; skip quietly.
        return;
    }
    let src = fs::read_to_string(&abs).unwrap();
    assert!(
        !src.contains("mpsc::Receiver<AgentEvent>"),
        "sdk/ide/stdio.rs must not take bounded Receiver<AgentEvent>"
    );
}
