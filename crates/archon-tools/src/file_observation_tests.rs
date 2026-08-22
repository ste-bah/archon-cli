//! Tests for the per-agent observation registry (#193 Phase A).
//!
//! Split from `file_observation.rs` to keep both files under the size gate.
//!
//! The registry no longer reads any filesystem itself (#201 Phase 1) — the
//! caller supplies the version, because only it knows which world it read.
//! These drive it through [`LocalFs`] rather than a hand-rolled token, so what
//! they exercise is the same derivation production uses.

use super::*;
use crate::filesystem::{FileSystem, LocalFs};

fn observer(name: &str) -> Observer {
    Observer::new(name, None)
}

/// What the host says about `path` right now.
async fn seen(path: &Path) -> Option<FileVersion> {
    LocalFs.version(path).await
}

/// Record the host's current view of `path`, as a tool would after showing it.
async fn record(registry: &ObservationRegistry, observer: &Observer, path: &Path) {
    let observation = seen(path)
        .await
        .map_or(Observation::Absent, Observation::Present);
    registry.record_as(observer, path, observation);
}

/// Judge a pending write against the host's current view.
async fn verdict(registry: &ObservationRegistry, observer: &Observer, path: &Path) -> Verdict {
    registry.verdict(observer, path, seen(path).await)
}

#[tokio::test]
async fn a_path_nobody_looked_at_is_unobserved() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");

    assert_eq!(
        verdict(&registry, &observer("s"), &file).await,
        Verdict::Unobserved
    );
}

#[tokio::test]
async fn an_unchanged_file_reads_fresh() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");

    record(&registry, &observer("s"), &file).await;

    assert_eq!(
        verdict(&registry, &observer("s"), &file).await,
        Verdict::Fresh
    );
}

/// The failure this exists to catch: something else changed the file
/// between the read and the write.
#[tokio::test]
async fn an_externally_modified_file_reads_stale() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "fn main() {}").expect("write");
    record(&registry, &observer("s"), &file).await;

    std::fs::write(&file, "fn main() { changed_underneath() }").expect("rewrite");

    match verdict(&registry, &observer("s"), &file).await {
        Verdict::Stale { detail } => assert!(detail.contains("modified"), "{detail}"),
        other => panic!("expected stale, got {other:?}"),
    }
}

#[tokio::test]
async fn a_deleted_file_reads_stale() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "x").expect("write");
    record(&registry, &observer("s"), &file).await;
    std::fs::remove_file(&file).expect("remove");

    match verdict(&registry, &observer("s"), &file).await {
        Verdict::Stale { detail } => assert!(detail.contains("deleted"), "{detail}"),
        other => panic!("expected stale, got {other:?}"),
    }
}

/// "I checked and it was not there" is evidence, and a file appearing
/// since contradicts it.
#[tokio::test]
async fn a_file_that_appeared_after_a_negative_observation_reads_stale() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("new.rs");

    record(&registry, &observer("s"), &file).await;
    assert_eq!(
        registry.observation(&observer("s"), &file),
        Some(Observation::Absent)
    );
    assert_eq!(
        verdict(&registry, &observer("s"), &file).await,
        Verdict::Fresh
    );

    std::fs::write(&file, "someone else made it").expect("write");

    match verdict(&registry, &observer("s"), &file).await {
        Verdict::Stale { detail } => assert!(detail.contains("did not exist"), "{detail}"),
        other => panic!("expected stale, got {other:?}"),
    }
}

/// A parent's read is not evidence for a child that never looked.
#[tokio::test]
async fn a_subagent_does_not_inherit_its_parents_observations() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "x").expect("write");

    let parent = Observer::new("session-1", None);
    let child = Observer::new("session-1", Some("agent-7"));
    record(&registry, &parent, &file).await;

    assert_eq!(verdict(&registry, &parent, &file).await, Verdict::Fresh);
    assert_eq!(
        verdict(&registry, &child, &file).await,
        Verdict::Unobserved,
        "session_id is copied verbatim to children, so keying on it alone \
         would hand a subagent evidence it never gathered"
    );
}

#[tokio::test]
async fn two_spellings_of_one_path_are_the_same_observation() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("src")).expect("mkdir");
    let file = dir.path().join("src").join("a.rs");
    std::fs::write(&file, "x").expect("write");

    record(&registry, &observer("s"), &file).await;
    let indirect = dir.path().join("src").join(".").join("a.rs");

    assert_eq!(
        verdict(&registry, &observer("s"), &indirect).await,
        Verdict::Fresh
    );
}

#[tokio::test]
async fn ending_a_session_forgets_it_and_its_subagents() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "x").expect("write");

    let parent = Observer::new("session-1", None);
    let child = Observer::new("session-1", Some("agent-7"));
    let other = Observer::new("session-2", None);
    record(&registry, &parent, &file).await;
    record(&registry, &child, &file).await;
    record(&registry, &other, &file).await;

    registry.forget_session("session-1");

    assert!(registry.is_empty(&parent));
    assert!(registry.is_empty(&child));
    assert_eq!(registry.len(&other), 1, "another session was collateral");
}

/// A subagent ending must take its own record with it and nothing else.
///
/// The parent is still running when a child finishes, and it holds readings
/// behind edits it has not made yet. Widening this to `forget_session` would
/// flip those from `Fresh` to `Unobserved` and — under the default
/// `read_before_edit = "block"` — refuse a write the parent had every right to
/// make, at a moment determined by whichever child happened to finish.
#[tokio::test]
async fn an_agent_can_be_forgotten_without_touching_its_parent_or_siblings() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "x").expect("write");

    let parent = Observer::new("session-1", None);
    let finished = Observer::new("session-1", Some("agent-7"));
    let sibling = Observer::new("session-1", Some("agent-8"));
    record(&registry, &parent, &file).await;
    record(&registry, &finished, &file).await;
    record(&registry, &sibling, &file).await;

    registry.forget_agent(&finished);

    assert!(registry.is_empty(&finished), "the agent that ended is gone");
    assert_eq!(
        verdict(&registry, &parent, &file).await,
        Verdict::Fresh,
        "the parent's reading must survive its child ending, or its next edit \
         is refused for a file it has read"
    );
    assert_eq!(
        verdict(&registry, &sibling, &file).await,
        Verdict::Fresh,
        "a sibling still running must survive too"
    );
}

/// The token is compared, never parsed. This pins that two different files
/// do not collide on it, without asserting anything about its shape.
#[tokio::test]
async fn versions_of_different_content_differ() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");
    std::fs::write(&file, "short").expect("write");
    let first = seen(&file).await.expect("present");
    std::fs::write(&file, "considerably longer content").expect("rewrite");
    let second = seen(&file).await.expect("present");

    assert_ne!(first, second);
}

/// The key must not depend on whether the file is there, or the two
/// transitions this module exists to catch both report "never looked".
#[tokio::test]
async fn an_observation_survives_the_file_appearing_and_vanishing() {
    let registry = ObservationRegistry::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a.rs");

    record(&registry, &observer("s"), &file).await;
    assert_eq!(registry.len(&observer("s")), 1);

    std::fs::write(&file, "x").expect("write");
    assert_ne!(
        verdict(&registry, &observer("s"), &file).await,
        Verdict::Unobserved,
        "the observation was lost when the file appeared"
    );

    record(&registry, &observer("s"), &file).await;
    std::fs::remove_file(&file).expect("remove");
    assert_ne!(
        verdict(&registry, &observer("s"), &file).await,
        Verdict::Unobserved,
        "the observation was lost when the file vanished"
    );
    assert_eq!(
        registry.len(&observer("s")),
        1,
        "one file must not occupy two keys"
    );
}

#[tokio::test]
async fn a_missing_file_has_no_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert_eq!(seen(&dir.path().join("nope")).await, None);
}
