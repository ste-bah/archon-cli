//! Tests for the session-surface projection (#200 Phase 4).
//!
//! Segments are closed and summarised through the same store calls the
//! runtime uses (`close_compaction_segment` → `claim_…` → `complete_…`), so
//! what is folded here is what a real session leaves behind.

use super::*;

use crate::storage::SessionStore;

struct Fixture {
    _dir: tempfile::TempDir,
    store: SessionStore,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::open(&dir.path().join("sessions.db")).expect("session store");
    Fixture { _dir: dir, store }
}

fn message(role: &str, text: &str) -> String {
    serde_json::json!({ "role": role, "content": text }).to_string()
}

fn seed(fixture: &Fixture, count: usize) -> String {
    let session = fixture
        .store
        .create_session("/tmp/source", None, "test-model")
        .expect("create session");
    for index in 0..count {
        fixture
            .store
            .save_message(
                &session.id,
                index as u64,
                &message("user", &format!("message-number-{index}")),
            )
            .expect("save message");
    }
    session.id
}

/// Close a segment over `start..=end` and summarise it, exactly the way the
/// agent's staged compaction does.
fn compact_span(fixture: &Fixture, session_id: &str, start: u64, end: u64, summary: &str) {
    let messages = fixture.store.load_messages(session_id).expect("load");
    let body = messages[start as usize..=end as usize].to_vec();
    let segment = fixture
        .store
        .close_compaction_segment(session_id, start, end, &body)
        .expect("close segment");
    let claim = fixture
        .store
        .claim_compaction_segment_summary(&segment.id, "test-model", "test-attribution")
        .expect("claim")
        .expect("segment was claimable");
    assert!(
        fixture
            .store
            .complete_compaction_segment_summary(&segment.id, &claim, summary, 1, 1, 0.0)
            .expect("complete"),
        "the summary must land on the segment"
    );
}

#[test]
fn a_session_that_never_compacted_surfaces_its_whole_log() {
    let fixture = fixture();
    let id = seed(&fixture, 5);

    let surface = session_surface(&fixture.store, &id).expect("surface");

    assert_eq!(surface.messages_total, 5);
    assert_eq!(surface.replaced_messages, 0);
    assert_eq!(surface.messages.len(), 5);
    assert!(surface.messages.iter().all(|entry| !entry.compacted));
    assert!(surface.messages[0].payload.contains("message-number-0"));
}

/// The whole point. A span the source session compacted away is still in its
/// log, and must not be on its surface.
#[test]
fn a_compacted_span_leaves_the_log_alone_and_the_surface_without_it() {
    let fixture = fixture();
    let id = seed(&fixture, 8);
    compact_span(
        &fixture,
        &id,
        0,
        4,
        "they argued about indexes and settled on cozo",
    );

    // The log still has every word of it — that is the trap.
    let log = fixture.store.load_messages(&id).expect("load");
    assert_eq!(log.len(), 8);
    assert!(log[0].contains("message-number-0"));

    let surface = session_surface(&fixture.store, &id).expect("surface");

    assert_eq!(surface.messages_total, 8);
    assert_eq!(surface.replaced_messages, 5);
    // One stand-in plus messages 5, 6, 7.
    assert_eq!(surface.messages.len(), 4);
    assert!(surface.messages[0].compacted);
    assert!(
        surface.messages[0]
            .payload
            .contains("they argued about indexes")
    );
    for dropped in 0..5 {
        let needle = format!("message-number-{dropped}");
        assert!(
            !surface
                .messages
                .iter()
                .any(|entry| entry.payload.contains(&needle)),
            "{needle} is compacted away but still on the surface"
        );
    }
    assert!(surface.messages[1].payload.contains("message-number-5"));
}

/// The rule from the projection module doc: a unit with nothing to add hands
/// back the `Arc` it was given.
#[test]
fn a_message_inside_a_segment_past_its_head_returns_the_same_state() {
    let fixture = fixture();
    let id = seed(&fixture, 8);
    compact_span(&fixture, &id, 0, 4, "summary");
    let unit = SessionSurfaceProjection::for_session(&fixture.store, &id).expect("unit");

    let head = unit.apply(
        Arc::new(unit.init()),
        &SessionEvent {
            seq: 0,
            payload: "irrelevant".into(),
        },
    );
    let after = unit.apply(
        Arc::clone(&head),
        &SessionEvent {
            seq: 1,
            payload: "irrelevant".into(),
        },
    );

    assert!(
        Arc::ptr_eq(&head, &after),
        "a message already covered by a stand-in changed the surface"
    );
    assert_eq!(head.entries.len(), 1);
}

/// The cache is keyed by a `&'static str`, so nothing in the key records the
/// segments. Without the fingerprint check, a compaction that happened after
/// the cache was written would be invisible forever.
#[test]
fn a_segment_closed_after_the_cache_was_written_is_not_served_stale() {
    let fixture = fixture();
    let id = seed(&fixture, 8);

    let before = session_surface(&fixture.store, &id).expect("first surface");
    assert_eq!(before.messages.len(), 8);

    compact_span(&fixture, &id, 0, 4, "settled on cozo");

    let after = session_surface(&fixture.store, &id).expect("second surface");
    assert_eq!(after.messages.len(), 4);
    assert_eq!(after.replaced_messages, 5);
}

/// `replace_messages` after a log-level compaction shrinks the log under a
/// cache the driver resumes from, and the driver resumes *after* the cached
/// seq so it never revisits the gap.
///
/// Since the follow-up fix this is satisfied twice over: `replace_messages`
/// now drops the session's projection caches outright, and this unit's own
/// length check would catch it anyway. The assertion is on the observable
/// result rather than on which of the two did the work, so it holds if either
/// is removed — the store-level fix has its own mutation coverage in
/// `crates/archon-session/tests/session_projection.rs`.
#[test]
fn a_log_that_shrank_under_the_cache_is_refolded_rather_than_served() {
    let fixture = fixture();
    let id = seed(&fixture, 8);

    let before = session_surface(&fixture.store, &id).expect("first surface");
    assert_eq!(before.messages.len(), 8);

    let kept = vec![
        message("user", "[Context Summary] the first seven, distilled"),
        message("user", "message-number-7"),
    ];
    fixture
        .store
        .replace_messages(&id, &kept)
        .expect("replace messages");

    let after = session_surface(&fixture.store, &id).expect("second surface");
    assert_eq!(after.messages_total, 2);
    assert_eq!(after.messages.len(), 2);
    assert!(after.messages[0].payload.contains("distilled"));
}

#[test]
fn a_segment_with_no_summary_names_the_span_rather_than_reproducing_it() {
    let fixture = fixture();
    let id = seed(&fixture, 8);
    let log = fixture.store.load_messages(&id).expect("load");
    // Closed but never summarised: the source has decided the span is out of
    // its context, and there is nothing to put in its place but a note.
    fixture
        .store
        .close_compaction_segment(&id, 0, 4, &log[0..=4])
        .expect("close segment");

    let surface = session_surface(&fixture.store, &id).expect("surface");

    assert_eq!(surface.replaced_messages, 5);
    assert!(surface.messages[0].payload.contains("no summary is stored"));
    assert!(!surface.messages[0].payload.contains("message-number-0"));
}

/// The persisted cache requires it.
#[test]
fn the_state_round_trips_through_json() {
    let fixture = fixture();
    let id = seed(&fixture, 8);
    compact_span(&fixture, &id, 0, 4, "summary");
    let unit = SessionSurfaceProjection::for_session(&fixture.store, &id).expect("unit");
    let projected = fixture.store.project(&id, &unit).expect("project");

    let encoded = serde_json::to_string(&*projected.state).expect("serialise");
    let decoded: SessionSurfaceState = serde_json::from_str(&encoded).expect("deserialise");

    assert_eq!(decoded, *projected.state);
}

#[test]
fn the_view_names_what_a_client_needs() {
    let fixture = fixture();
    let id = seed(&fixture, 8);
    compact_span(&fixture, &id, 0, 4, "summary");
    let unit = SessionSurfaceProjection::for_session(&fixture.store, &id).expect("unit");
    let projected = fixture.store.project(&id, &unit).expect("project");

    let view = unit.view(&projected.state);
    assert_eq!(view["entries"], 4);
    assert_eq!(view["live"], 3);
    assert_eq!(view["compacted_spans"], 1);
    assert_eq!(view["replaced_messages"], 5);
}
