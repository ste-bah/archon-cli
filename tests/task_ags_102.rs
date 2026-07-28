//! Bounded Agent event transport source contracts.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    let path = repo_root().join(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn agent_owns_bounded_timestamped_event_sender() {
    let src = read("crates/archon-core/src/agent.rs");
    assert!(
        src.contains("mpsc::Sender<TimestampedEvent>"),
        "Agent must own a bounded TimestampedEvent sender"
    );
    assert!(!src.contains("UnboundedSender<TimestampedEvent>"));
}

#[test]
fn agent_event_send_awaits_capacity() {
    let src = read("crates/archon-core/src/agent/events.rs");
    assert!(
        src.contains("self.event_tx.send(timestamped).await"),
        "Agent event emission must await bounded capacity"
    );
}

#[test]
fn production_constructors_use_shared_capacity() {
    for path in [
        "src/session/build_agent.rs",
        "src/session/interactive_agent.rs",
        "crates/archon-core/src/orchestrator.rs",
    ] {
        let src = read(path);
        assert!(
            src.contains("AGENT_EVENT_CHANNEL_CAPACITY"),
            "{path} must use shared Agent event capacity"
        );
        assert!(
            !src.contains("unbounded_channel::<TimestampedEvent>"),
            "{path} must not create unbounded TimestampedEvent transport"
        );
    }
}

#[test]
fn print_and_ide_consumers_use_bounded_receivers() {
    for path in [
        "crates/archon-core/src/print_mode.rs",
        "crates/archon-sdk/src/ide/stdio.rs",
    ] {
        let src = read(path);
        assert!(src.contains("mpsc::Receiver<TimestampedEvent>"), "{path}");
        assert!(
            !src.contains("UnboundedReceiver<TimestampedEvent>"),
            "{path}"
        );
    }
}
