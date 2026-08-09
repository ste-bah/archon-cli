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
    // Each entry is one construction site. The orchestrator spans two files
    // since the file-size split moved its agent construction into
    // `orchestrator_executor.rs`, so the contract is checked per site, not
    // per file.
    for paths in [
        &["src/session/build_agent.rs"][..],
        &["src/session/interactive_agent.rs"][..],
        &[
            "crates/archon-core/src/orchestrator.rs",
            "crates/archon-core/src/orchestrator_executor.rs",
        ][..],
    ] {
        let site = paths.join(", ");
        let src: String = paths.iter().map(|path| read(path)).collect();
        assert!(
            src.contains("AGENT_EVENT_CHANNEL_CAPACITY"),
            "{site} must use shared Agent event capacity"
        );
        assert!(
            !src.contains("unbounded_channel::<TimestampedEvent>"),
            "{site} must not create unbounded TimestampedEvent transport"
        );
    }
}

/// Both consumers that drain an agent's event stream must do so through a
/// bounded receiver, so a slow reader applies backpressure rather than letting
/// the queue grow without limit.
///
/// The payload differs by consumer, which is why the type is paired with the
/// path rather than assumed. `print_mode` drains `TimestampedEvent` directly;
/// the IDE transport drains notifications already mapped for the wire, because
/// routing everything through one channel is what keeps stdout single-writer
/// (#26). Boundedness is the property under test — the payload type is not.
#[test]
fn print_and_ide_consumers_use_bounded_receivers() {
    for (path, payload) in [
        ("crates/archon-core/src/print_mode.rs", "TimestampedEvent"),
        ("crates/archon-sdk/src/ide/stdio.rs", "JRpcNotification"),
    ] {
        let src = read(path);
        assert!(
            src.contains(&format!("mpsc::Receiver<{payload}>")),
            "{path} must drain {payload} through a bounded receiver"
        );
        assert!(
            !src.contains(&format!("UnboundedReceiver<{payload}>")),
            "{path} must not drain {payload} through an unbounded receiver"
        );
    }
}
