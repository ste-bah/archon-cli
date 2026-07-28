//! Preserve gate for bounded Agent event transport.

use std::fs;
use std::path::PathBuf;

fn tui_source(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(path);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

#[test]
fn dispatcher_keeps_bounded_timestamped_event_sender() {
    let source = tui_source("task_dispatch.rs");
    assert!(
        source.contains("mpsc::Sender<TimestampedEvent>"),
        "AgentDispatcher must retain bounded TimestampedEvent transport"
    );
    assert!(!source.contains("UnboundedSender<TimestampedEvent>"));
}

#[test]
fn dispatcher_prompt_path_remains_nonblocking() {
    let source = tui_source("task_dispatch.rs");
    let start = source
        .find("pub fn spawn_turn(")
        .expect("spawn_turn function");
    let end = source[start..]
        .find("fn spawn_turn_internal(")
        .map(|offset| start + offset)
        .expect("spawn_turn_internal boundary");
    assert!(
        !source[start..end].contains(".await"),
        "input dispatch must not await agent work or event capacity"
    );
}
