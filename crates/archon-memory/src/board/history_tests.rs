//! The decline invariant and the transition history that carries it.
//!
//! Split from `board_tests.rs` to hold the 500-line ceiling; the seam is the
//! obvious one -- everything here is about `board_item_events` and the one
//! transition that cannot be made without writing to it.

use super::{BoardStatus, board_tests::new_item};
use crate::graph::MemoryGraph;
use crate::types::MemoryError;

/// The invariant the drain gate depends on has to hold at the STORE, because by
/// the time the gate reads the row the agent that could have justified the
/// decline is gone. There must be no call that reaches `declined` without one.
#[test]
fn declining_without_a_reason_is_refused_by_the_store() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "not worth doing"))
        .expect("create");

    // The general transition is the back door, and it is shut.
    let refused = graph
        .set_board_item_status(&item.id, BoardStatus::Open, BoardStatus::Declined)
        .expect_err("set_board_item_status must not be able to reach declined");
    assert!(
        matches!(&refused, MemoryError::Database(message) if message.contains("decline_board_item")),
        "the refusal must point at the call that carries a reason, got: {refused}"
    );

    for blank in ["", "   ", "\n\t"] {
        let error = graph
            .decline_board_item(&item.id, BoardStatus::Open, blank)
            .expect_err("a blank reason must be refused");
        assert!(
            matches!(&error, MemoryError::Database(message) if message.contains("reason")),
            "the refusal must name the missing field, got: {error}"
        );
    }

    assert_eq!(
        graph.get_board_item(&item.id).expect("get").status,
        BoardStatus::Open,
        "every refused decline must leave the item exactly as it was"
    );
    assert!(
        graph
            .board_item_history(&item.id)
            .expect("history")
            .is_empty(),
        "a refused decline must record nothing"
    );
}

#[test]
fn a_decline_round_trips_with_its_reason() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "out of scope"))
        .expect("create");
    let reason = "the caller already validates this upstream; a second check would drift";

    let declined = graph
        .decline_board_item(&item.id, BoardStatus::Open, reason)
        .expect("decline");
    assert!(declined.applied);
    assert_eq!(declined.item.status, BoardStatus::Declined);
    assert_eq!(declined.item.decline_reason.as_deref(), Some(reason));

    assert_eq!(
        graph.get_board_item(&item.id).expect("get").decline_reason,
        Some(reason.to_string()),
        "the reason must come back on a fresh read, not just on the write's own answer"
    );
    let listed = graph
        .list_board_items_by_run("run-1", &[])
        .expect("list")
        .pop()
        .expect("one item");
    assert_eq!(listed.decline_reason.as_deref(), Some(reason));
}

/// A decline is still a compare-and-set: a caller working from a stale read must
/// not close an item on a verdict it never saw.
#[test]
fn a_decline_from_the_wrong_prior_status_changes_nothing() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "contested"))
        .expect("create");
    graph.claim_board_item(&item.id, "agent-a").expect("claim");

    let stale = graph
        .decline_board_item(&item.id, BoardStatus::Open, "I think this is unnecessary")
        .expect("stale decline");
    assert!(!stale.applied);
    assert_eq!(stale.item.status, BoardStatus::Claimed);
    assert_eq!(stale.item.decline_reason, None);
    assert!(
        graph
            .board_item_history(&item.id)
            .expect("history")
            .is_empty(),
        "a refused compare-and-set must not leave a history entry claiming it happened"
    );
}

/// What the history is FOR: reading back the ladder an item climbed, in order,
/// with the reason attached to the step that needed one.
#[test]
fn the_history_records_every_transition_in_order() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-ladder", "escalating"))
        .expect("create");
    graph.claim_board_item(&item.id, "agent-a").expect("claim");
    graph
        .set_board_item_status(&item.id, BoardStatus::Claimed, BoardStatus::InReview)
        .expect("to review");
    graph
        .set_board_item_status(&item.id, BoardStatus::InReview, BoardStatus::GapsRemain)
        .expect("gaps");
    graph
        .decline_board_item(&item.id, BoardStatus::GapsRemain, "the gap is in the spec")
        .expect("decline");

    let history = graph.board_item_history(&item.id).expect("history");
    assert_eq!(
        history
            .iter()
            .map(|event| (event.seq, event.from_status, event.to_status))
            .collect::<Vec<_>>(),
        vec![
            (0, BoardStatus::Claimed, BoardStatus::InReview),
            (1, BoardStatus::InReview, BoardStatus::GapsRemain),
            (2, BoardStatus::GapsRemain, BoardStatus::Declined),
        ],
        "the ladder must read back in the order it was climbed"
    );
    for event in &history {
        assert_eq!(event.run_id, "run-ladder");
        assert_eq!(event.round, 0);
        assert_eq!(
            event.actor.as_deref(),
            Some("agent-a"),
            "a transition is attributed to whoever held the item at the time"
        );
    }
    assert_eq!(history[0].note, "");
    assert_eq!(history[2].note, "the gap is in the spec");
    assert!(history[0].at <= history[2].at);
}

/// A parent can reopen a declined item and the second decline can differ. The
/// item is closed on the standing reason, not on the first one ever given.
#[test]
fn the_standing_decline_reason_is_the_latest_one() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "twice declined"))
        .expect("create");
    graph
        .decline_board_item(&item.id, BoardStatus::Open, "first pass: looks intentional")
        .expect("first decline");
    graph
        .set_board_item_status(&item.id, BoardStatus::Declined, BoardStatus::Open)
        .expect("reopen");
    graph
        .decline_board_item(
            &item.id,
            BoardStatus::Open,
            "second pass: covered by the new test",
        )
        .expect("second decline");

    assert_eq!(
        graph
            .get_board_item(&item.id)
            .expect("get")
            .decline_reason
            .as_deref(),
        Some("second pass: covered by the new test")
    );
    assert_eq!(
        graph.board_item_history(&item.id).expect("history").len(),
        3,
        "the superseded decline must still be on the record"
    );
}
