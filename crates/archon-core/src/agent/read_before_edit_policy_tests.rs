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
use archon_tools::filesystem::{FileMeta, FileSystem, local_fs};
use archon_tools::tool::ToolContext;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

fn blocking() -> FilesystemConfig {
    FilesystemConfig {
        read_before_edit: ReadBeforeEdit::Block,
    }
}

/// The world these tests write their fixtures into: the host.
fn host() -> Arc<dyn FileSystem> {
    local_fs()
}

/// A fresh session id per test: the registry is process-global and these run
/// concurrently.
fn observer(tag: &str) -> Observer {
    Observer::new(&format!("{tag}-{}", uuid::Uuid::new_v4()), None)
}

fn edit_of(path: &std::path::Path) -> serde_json::Value {
    serde_json::json!({ "file_path": path.display().to_string() })
}

#[tokio::test]
async fn editing_a_file_never_read_is_refused_with_something_actionable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");

    let reason = refusal_for(
        blocking(),
        host().as_ref(),
        &observer("never-read"),
        "Edit",
        &edit_of(&file),
    )
    .await
    .expect("refused");

    assert!(reason.contains("have not read"), "{reason}");
    assert!(reason.contains("Read it first"), "{reason}");
    assert!(
        reason.contains("a.rs"),
        "the message must name the file: {reason}"
    );
}

#[tokio::test]
async fn editing_a_file_modified_since_the_read_is_refused_as_stale() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");
    let observer = observer("stale");

    record(
        blocking(),
        host().as_ref(),
        &observer,
        "Read",
        &edit_of(&file),
        true,
    )
    .await;
    assert_eq!(
        refusal_for(
            blocking(),
            host().as_ref(),
            &observer,
            "Edit",
            &edit_of(&file)
        )
        .await,
        None,
        "a read of the current bytes must permit the edit"
    );

    std::fs::write(&file, "fn main() { someone_else_was_here() }").expect("rewrite");

    let reason = refusal_for(
        blocking(),
        host().as_ref(),
        &observer,
        "Edit",
        &edit_of(&file),
    )
    .await
    .expect("refused as stale");
    assert!(reason.contains("modified since"), "{reason}");
    assert!(reason.contains("Read it again"), "{reason}");
}

/// "I checked and it was not there" is a real observation, and a file appearing
/// in the meantime contradicts it.
#[tokio::test]
async fn writing_over_a_file_that_appeared_after_a_negative_observation_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("new.rs");
    let observer = observer("negative");

    record(
        blocking(),
        host().as_ref(),
        &observer,
        "Read",
        &edit_of(&file),
        true,
    )
    .await;
    assert_eq!(
        refusal_for(
            blocking(),
            host().as_ref(),
            &observer,
            "Write",
            &edit_of(&file)
        )
        .await,
        None,
        "creating a file confirmed absent must be allowed"
    );

    std::fs::write(&file, "another agent got there first").expect("write");

    let reason = refusal_for(
        blocking(),
        host().as_ref(),
        &observer,
        "Write",
        &edit_of(&file),
    )
    .await
    .expect("refused");
    assert!(reason.contains("did not exist"), "{reason}");
}

/// The policy has to be removable, not merely quiet — `off` must not consult
/// the registry at all.
#[tokio::test]
async fn off_restores_the_previous_behaviour_exactly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");
    let observer = observer("off");
    let off = FilesystemConfig {
        read_before_edit: ReadBeforeEdit::Off,
    };

    assert_eq!(
        refusal_for(off, host().as_ref(), &observer, "Edit", &edit_of(&file)).await,
        None,
        "an unread file must be editable with the policy off"
    );

    record(
        off,
        host().as_ref(),
        &observer,
        "Read",
        &edit_of(&file),
        true,
    )
    .await;
    assert!(
        FILE_OBSERVATIONS.is_empty(&observer),
        "with the policy off nothing should even be recorded"
    );
}

/// Warn lets the write through. What is pinned here is that it proceeds where
/// Block would have stopped it; asserting on the log line would be testing
/// tracing.
#[tokio::test]
async fn warn_allows_the_write_that_block_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");
    let observer = observer("warn");
    let warn = FilesystemConfig {
        read_before_edit: ReadBeforeEdit::Warn,
    };

    assert!(
        refusal_for(
            blocking(),
            host().as_ref(),
            &observer,
            "Edit",
            &edit_of(&file)
        )
        .await
        .is_some()
    );
    assert_eq!(
        refusal_for(warn, host().as_ref(), &observer, "Edit", &edit_of(&file)).await,
        None
    );
}

/// A parent's read is not evidence for a child that never looked. The
/// distinction rides on `subagent_id`, because `session_id` is copied verbatim
/// into children.
#[tokio::test]
async fn a_subagent_gets_its_own_registry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");

    let session = format!("session-{}", uuid::Uuid::new_v4());
    let parent = Observer::new(&session, None);
    let child = Observer::new(&session, Some("agent-7"));

    record(
        blocking(),
        host().as_ref(),
        &parent,
        "Read",
        &edit_of(&file),
        true,
    )
    .await;

    assert_eq!(
        refusal_for(
            blocking(),
            host().as_ref(),
            &parent,
            "Edit",
            &edit_of(&file)
        )
        .await,
        None,
        "the agent that read it may edit it"
    );
    assert!(
        refusal_for(blocking(), host().as_ref(), &child, "Edit", &edit_of(&file))
            .await
            .is_some(),
        "a subagent must not inherit its parent's reading"
    );
}

/// An agent's own edit must not lock it out of its next one.
#[tokio::test]
async fn a_second_edit_by_the_same_agent_is_allowed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "one").expect("write");
    let observer = observer("second-edit");

    record(
        blocking(),
        host().as_ref(),
        &observer,
        "Read",
        &edit_of(&file),
        true,
    )
    .await;
    std::fs::write(&file, "two").expect("the edit itself");
    record(
        blocking(),
        host().as_ref(),
        &observer,
        "Edit",
        &edit_of(&file),
        true,
    )
    .await;

    assert_eq!(
        refusal_for(
            blocking(),
            host().as_ref(),
            &observer,
            "Edit",
            &edit_of(&file)
        )
        .await,
        None,
        "an agent's own write must refresh what it knows"
    );
}

/// A failed read is not a sighting.
#[tokio::test]
async fn a_failed_read_records_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "x").expect("write");
    let observer = observer("failed-read");

    record(
        blocking(),
        host().as_ref(),
        &observer,
        "Read",
        &edit_of(&file),
        false,
    )
    .await;

    assert!(
        refusal_for(
            blocking(),
            host().as_ref(),
            &observer,
            "Edit",
            &edit_of(&file)
        )
        .await
        .is_some()
    );
}

/// A shell command names no path to check, so guarding it would refuse work it
/// cannot describe. The partial guarantee is deliberate.
#[tokio::test]
async fn bash_is_not_guarded() {
    let observer = observer("bash");
    let input = serde_json::json!({ "command": "echo hi > a.rs" });

    assert_eq!(
        refusal_for(blocking(), host().as_ref(), &observer, "Bash", &input).await,
        None
    );
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

/// A world that is not the host, whose files change independently of it.
///
/// Only `metadata` is answered. Everything else refuses rather than returning
/// a plausible empty success: if a future test starts depending on one of them
/// it should fail loudly, not silently pass against a fiction.
#[derive(Debug)]
struct OtherWorld {
    len: AtomicU64,
}

fn elsewhere() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "OtherWorld answers metadata only",
    )
}

#[async_trait::async_trait]
impl FileSystem for OtherWorld {
    async fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(elsewhere())
    }

    async fn write(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Err(elsewhere())
    }

    async fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
        Err(elsewhere())
    }

    async fn metadata(&self, _path: &Path) -> io::Result<FileMeta> {
        Ok(FileMeta {
            len: self.len.load(Ordering::SeqCst),
            modified_nanos: Some(1),
            is_dir: false,
        })
    }

    async fn read_dir(&self, _path: &Path) -> io::Result<Vec<PathBuf>> {
        Err(elsewhere())
    }

    async fn remove_file(&self, _path: &Path) -> io::Result<()> {
        Err(elsewhere())
    }

    async fn rename(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        Err(elsewhere())
    }

    fn rerooted(self: Arc<Self>, _working_dir: &Path) -> Arc<dyn FileSystem> {
        self
    }

    async fn glob(&self, _base: &Path, _pattern: &str) -> io::Result<Vec<PathBuf>> {
        Err(elsewhere())
    }
}

/// The point of #201 Phase 1: the guard judges the world the write lands in.
///
/// The host file is written once and never touched again, so a guard that
/// still consulted the host would report `Fresh` throughout. Only a guard
/// reading the supplied world sees the change and refuses.
#[tokio::test]
async fn the_freshness_token_comes_from_the_world_not_the_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("remote.rs");
    std::fs::write(&file, "the host copy, never modified").expect("write");
    let observer = observer("other-world");
    let world = OtherWorld {
        len: AtomicU64::new(10),
    };

    record(blocking(), &world, &observer, "Read", &edit_of(&file), true).await;
    assert_eq!(
        refusal_for(blocking(), &world, &observer, "Edit", &edit_of(&file)).await,
        None,
        "the version it read is still the version that is there"
    );

    // The file changes in that world alone. The host copy is untouched.
    world.len.store(11, Ordering::SeqCst);

    let reason = refusal_for(blocking(), &world, &observer, "Edit", &edit_of(&file))
        .await
        .expect("a change in the execution world must be seen");
    assert!(reason.contains("modified since"), "{reason}");
}
