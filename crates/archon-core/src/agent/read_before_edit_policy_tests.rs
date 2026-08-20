//! The acceptance criteria for #193 Phase A, against the real policy.
//!
//! In-crate rather than under `tests/` so `tool_preflight_freshness` can stay
//! `pub(crate)`. Widening a module to `pub` for the benefit of a test is what
//! disarmed `dead_code` across twelve screens in #189: a test is not a caller,
//! and the visibility should say who the callers are.
//!
//! These drive the free functions both tool loops call, rather than a
//! reimplementation of them, so what passes here is what runs.

use super::tool_preflight_freshness::{observer_for, record, refusal_for};
use crate::config::{FilesystemConfig, ReadBeforeEdit};
use archon_tools::file_observation::{FILE_OBSERVATIONS, Observer};
use archon_tools::tool::ToolContext;

fn blocking() -> FilesystemConfig {
    FilesystemConfig {
        read_before_edit: ReadBeforeEdit::Block,
    }
}

/// A fresh session id per test: the registry is process-global and these run
/// concurrently.
fn observer(tag: &str) -> Observer {
    Observer::new(&format!("{tag}-{}", uuid::Uuid::new_v4()), None)
}

fn edit_of(path: &std::path::Path) -> serde_json::Value {
    serde_json::json!({ "file_path": path.display().to_string() })
}

#[test]
fn editing_a_file_never_read_is_refused_with_something_actionable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");

    let reason =
        refusal_for(blocking(), &observer("never-read"), "Edit", &edit_of(&file)).expect("refused");

    assert!(reason.contains("have not read"), "{reason}");
    assert!(reason.contains("Read it first"), "{reason}");
    assert!(
        reason.contains("a.rs"),
        "the message must name the file: {reason}"
    );
}

#[test]
fn editing_a_file_modified_since_the_read_is_refused_as_stale() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");
    let observer = observer("stale");

    record(blocking(), &observer, "Read", &edit_of(&file), true);
    assert_eq!(
        refusal_for(blocking(), &observer, "Edit", &edit_of(&file)),
        None,
        "a read of the current bytes must permit the edit"
    );

    std::fs::write(&file, "fn main() { someone_else_was_here() }").expect("rewrite");

    let reason =
        refusal_for(blocking(), &observer, "Edit", &edit_of(&file)).expect("refused as stale");
    assert!(reason.contains("modified since"), "{reason}");
    assert!(reason.contains("Read it again"), "{reason}");
}

/// "I checked and it was not there" is a real observation, and a file appearing
/// in the meantime contradicts it.
#[test]
fn writing_over_a_file_that_appeared_after_a_negative_observation_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("new.rs");
    let observer = observer("negative");

    record(blocking(), &observer, "Read", &edit_of(&file), true);
    assert_eq!(
        refusal_for(blocking(), &observer, "Write", &edit_of(&file)),
        None,
        "creating a file confirmed absent must be allowed"
    );

    std::fs::write(&file, "another agent got there first").expect("write");

    let reason = refusal_for(blocking(), &observer, "Write", &edit_of(&file)).expect("refused");
    assert!(reason.contains("did not exist"), "{reason}");
}

/// The policy has to be removable, not merely quiet — `off` must not consult
/// the registry at all.
#[test]
fn off_restores_the_previous_behaviour_exactly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");
    let observer = observer("off");
    let off = FilesystemConfig {
        read_before_edit: ReadBeforeEdit::Off,
    };

    assert_eq!(
        refusal_for(off, &observer, "Edit", &edit_of(&file)),
        None,
        "an unread file must be editable with the policy off"
    );

    record(off, &observer, "Read", &edit_of(&file), true);
    assert!(
        FILE_OBSERVATIONS.is_empty(&observer),
        "with the policy off nothing should even be recorded"
    );
}

/// Warn lets the write through. What is pinned here is that it proceeds where
/// Block would have stopped it; asserting on the log line would be testing
/// tracing.
#[test]
fn warn_allows_the_write_that_block_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");
    let observer = observer("warn");
    let warn = FilesystemConfig {
        read_before_edit: ReadBeforeEdit::Warn,
    };

    assert!(refusal_for(blocking(), &observer, "Edit", &edit_of(&file)).is_some());
    assert_eq!(refusal_for(warn, &observer, "Edit", &edit_of(&file)), None);
}

/// A parent's read is not evidence for a child that never looked. The
/// distinction rides on `subagent_id`, because `session_id` is copied verbatim
/// into children.
#[test]
fn a_subagent_gets_its_own_registry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");

    let session = format!("session-{}", uuid::Uuid::new_v4());
    let parent = Observer::new(&session, None);
    let child = Observer::new(&session, Some("agent-7"));

    record(blocking(), &parent, "Read", &edit_of(&file), true);

    assert_eq!(
        refusal_for(blocking(), &parent, "Edit", &edit_of(&file)),
        None,
        "the agent that read it may edit it"
    );
    assert!(
        refusal_for(blocking(), &child, "Edit", &edit_of(&file)).is_some(),
        "a subagent must not inherit its parent's reading"
    );
}

/// An agent's own edit must not lock it out of its next one.
#[test]
fn a_second_edit_by_the_same_agent_is_allowed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "one").expect("write");
    let observer = observer("second-edit");

    record(blocking(), &observer, "Read", &edit_of(&file), true);
    std::fs::write(&file, "two").expect("the edit itself");
    record(blocking(), &observer, "Edit", &edit_of(&file), true);

    assert_eq!(
        refusal_for(blocking(), &observer, "Edit", &edit_of(&file)),
        None,
        "an agent's own write must refresh what it knows"
    );
}

/// A failed read is not a sighting.
#[test]
fn a_failed_read_records_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "x").expect("write");
    let observer = observer("failed-read");

    record(blocking(), &observer, "Read", &edit_of(&file), false);

    assert!(refusal_for(blocking(), &observer, "Edit", &edit_of(&file)).is_some());
}

/// A shell command names no path to check, so guarding it would refuse work it
/// cannot describe. The partial guarantee is deliberate.
#[test]
fn bash_is_not_guarded() {
    let observer = observer("bash");
    let input = serde_json::json!({ "command": "echo hi > a.rs" });

    assert_eq!(refusal_for(blocking(), &observer, "Bash", &input), None);
}

#[test]
fn the_observer_is_taken_from_the_tool_context() {
    let ctx = ToolContext {
        session_id: "session-9".into(),
        subagent_id: Some("agent-3".into()),
        ..Default::default()
    };

    assert_eq!(
        observer_for(&ctx),
        Observer::new("session-9", Some("agent-3"))
    );
}
