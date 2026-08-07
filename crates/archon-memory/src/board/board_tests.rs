use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use super::{BoardItemKind, BoardStatus, NewBoardItem};
use crate::graph::MemoryGraph;
use crate::types::MemoryError;

pub(super) fn new_item(run_id: &str, title: &str) -> NewBoardItem {
    NewBoardItem {
        id: None,
        run_id: run_id.to_string(),
        kind: BoardItemKind::Issue,
        title: title.to_string(),
        evidence: "crates/archon-memory/src/board.rs:1 -- observed on read".to_string(),
        acceptance: "the relation exists and carries real columns".to_string(),
        raised_by: "agent-a".to_string(),
    }
}

#[test]
fn create_round_trips_every_column() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let created = graph
        .create_board_item(&new_item("run-1", "carry the columns"))
        .expect("create");

    assert_eq!(created.status, BoardStatus::Open);
    assert_eq!(created.kind, BoardItemKind::Issue);
    assert_eq!(created.round, 0);
    assert_eq!(created.claimed_by, None);

    let fetched = graph.get_board_item(&created.id).expect("get");
    assert_eq!(fetched, created, "a stored item must read back identically");
}

/// An item with no file references is unactionable by whoever picks it up: the
/// agent that knew where to look is gone by then. Rejecting at the write is the
/// only place that rule can hold.
#[test]
fn create_rejects_an_item_with_no_evidence() {
    let graph = MemoryGraph::in_memory().expect("graph");
    for blank in ["", "   ", "\n\t"] {
        let mut item = new_item("run-1", "no evidence");
        item.evidence = blank.to_string();
        let error = graph
            .create_board_item(&item)
            .expect_err("an item without evidence must be refused, not stored");
        assert!(
            matches!(&error, MemoryError::Database(message) if message.contains("evidence")),
            "the refusal must name the missing field, got: {error}"
        );
    }
    assert!(
        graph
            .list_board_items_by_run("run-1", &[])
            .expect("list")
            .is_empty(),
        "a refused item must leave nothing behind"
    );
}

#[test]
fn create_refuses_to_overwrite_an_existing_id() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let mut first = new_item("run-1", "original");
    first.id = Some("fixed-id".to_string());
    graph.create_board_item(&first).expect("create");

    let mut second = new_item("run-1", "impostor");
    second.id = Some("fixed-id".to_string());
    second.raised_by = "agent-b".to_string();
    graph
        .create_board_item(&second)
        .expect_err("a colliding id must not silently replace another agent's item");

    assert_eq!(
        graph.get_board_item("fixed-id").expect("get").title,
        "original"
    );
}

/// Two claimants, one item, released together: exactly one must win, and the
/// loser must be TOLD it lost rather than left to infer it from a later read.
///
/// Repeated, because a single round proves nothing about a race -- and the run
/// where both threads were observed inside `claim_board_item` at once is what
/// makes this a race rather than two sequential calls.
#[test]
fn two_claimants_race_and_exactly_one_wins() {
    let graph = Arc::new(MemoryGraph::in_memory().expect("graph"));
    let mut observed_overlap = false;

    for round in 0..25 {
        let item = graph
            .create_board_item(&new_item("run-race", &format!("contended {round}")))
            .expect("create");
        let gate = Arc::new(Barrier::new(2));
        let inside = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let outcomes: Vec<(bool, String)> = std::thread::scope(|scope| {
            let handles: Vec<_> = ["agent-one", "agent-two"]
                .into_iter()
                .map(|agent| {
                    let graph = Arc::clone(&graph);
                    let gate = Arc::clone(&gate);
                    let inside = Arc::clone(&inside);
                    let peak = Arc::clone(&peak);
                    let id = item.id.clone();
                    scope.spawn(move || {
                        gate.wait();
                        let now_inside = inside.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now_inside, Ordering::SeqCst);
                        let update = graph.claim_board_item(&id, agent).expect("claim");
                        inside.fetch_sub(1, Ordering::SeqCst);
                        (update.applied, agent.to_string())
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("claimant thread"))
                .collect()
        });

        observed_overlap |= peak.load(Ordering::SeqCst) == 2;

        let winners: Vec<&str> = outcomes
            .iter()
            .filter(|(applied, _)| *applied)
            .map(|(_, agent)| agent.as_str())
            .collect();
        assert_eq!(
            winners.len(),
            1,
            "round {round}: exactly one claimant may be told it claimed, got {outcomes:?}"
        );

        let stored = graph.get_board_item(&item.id).expect("get");
        assert_eq!(
            stored.claimed_by.as_deref(),
            Some(winners[0]),
            "round {round}: the stored claim must belong to the caller that was told it won"
        );
        assert_eq!(stored.status, BoardStatus::Claimed);
    }

    assert!(
        observed_overlap,
        "no round ever had both claimants inside claim_board_item at once, so this \
         test never exercised a race"
    );
}

#[test]
fn a_second_claim_on_a_held_item_is_refused_even_by_the_holder() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "held"))
        .expect("create");

    assert!(
        graph
            .claim_board_item(&item.id, "agent-a")
            .expect("claim")
            .applied
    );
    let repeat = graph
        .claim_board_item(&item.id, "agent-a")
        .expect("reclaim");
    assert!(
        !repeat.applied,
        "re-claiming must report that nothing changed; a caller that reads `applied` \
         as `I hold it` would otherwise never learn it lost the item"
    );
    assert_eq!(repeat.item.claimed_by.as_deref(), Some("agent-a"));
}

#[test]
fn claiming_a_missing_item_is_not_found() {
    let graph = MemoryGraph::in_memory().expect("graph");
    assert!(matches!(
        graph.claim_board_item("nobody", "agent-a"),
        Err(MemoryError::NotFound(_))
    ));
}

#[test]
fn release_frees_the_item_and_reports_a_no_op_second_time() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "handed back"))
        .expect("create");
    graph.claim_board_item(&item.id, "agent-a").expect("claim");

    let released = graph.release_board_claim(&item.id).expect("release");
    assert!(released.applied);
    assert_eq!(released.item.claimed_by, None);
    assert_eq!(
        released.item.status,
        BoardStatus::Open,
        "a released claim returns the item to the pool"
    );

    assert!(
        !graph
            .release_board_claim(&item.id)
            .expect("second release")
            .applied,
        "releasing an unheld item must report that it did nothing"
    );

    assert!(
        graph
            .claim_board_item(&item.id, "agent-b")
            .expect("reclaim")
            .applied,
        "a released item must be claimable again"
    );
}

/// Releasing an agent is not the same as retracting the work it recorded, so a
/// status further along the lifecycle survives the release.
#[test]
fn release_leaves_a_reviewed_item_in_review() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "under review"))
        .expect("create");
    graph.claim_board_item(&item.id, "agent-a").expect("claim");
    graph
        .set_board_item_status(&item.id, BoardStatus::Claimed, BoardStatus::InReview)
        .expect("to review");

    let released = graph.release_board_claim(&item.id).expect("release");
    assert!(released.applied);
    assert_eq!(released.item.status, BoardStatus::InReview);
    assert_eq!(released.item.claimed_by, None);
}

#[test]
fn status_transition_is_refused_from_the_wrong_prior_status() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let item = graph
        .create_board_item(&new_item("run-1", "verdict"))
        .expect("create");
    graph.claim_board_item(&item.id, "agent-a").expect("claim");
    graph
        .set_board_item_status(&item.id, BoardStatus::Claimed, BoardStatus::InReview)
        .expect("to review");

    // The parent still believes the item is `claimed` and tries to escalate it.
    // Without the compare-and-set that write would erase a verdict it never saw.
    let stale = graph
        .set_board_item_status(&item.id, BoardStatus::Claimed, BoardStatus::Escalated)
        .expect("stale transition");
    assert!(
        !stale.applied,
        "a transition from a status that no longer holds must be refused"
    );
    assert_eq!(stale.item.status, BoardStatus::InReview);

    let fresh = graph
        .set_board_item_status(&item.id, BoardStatus::InReview, BoardStatus::Resolved)
        .expect("fresh transition");
    assert!(fresh.applied);
    assert_eq!(fresh.item.status, BoardStatus::Resolved);
}

#[test]
fn list_by_run_partitions_and_filters() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let mine = graph
        .create_board_item(&new_item("run-mine", "mine"))
        .expect("create");
    let also_mine = graph
        .create_board_item(&new_item("run-mine", "also mine"))
        .expect("create");
    graph
        .create_board_item(&new_item("run-other", "not mine"))
        .expect("create");

    let listed = graph
        .list_board_items_by_run("run-mine", &[])
        .expect("list");
    assert_eq!(
        listed
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![mine.id.as_str(), also_mine.id.as_str()],
        "a run must see its own items, oldest first, and nobody else's"
    );

    graph.claim_board_item(&mine.id, "agent-a").expect("claim");
    let open = graph
        .list_board_items_by_run("run-mine", &[BoardStatus::Open])
        .expect("list open");
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, also_mine.id);

    assert!(
        graph
            .list_board_items_by_run("run-absent", &[])
            .expect("list")
            .is_empty()
    );
}

/// Reopening a store re-runs `init_schema`, so the relation and its index must
/// both tolerate already being there -- otherwise the board would work exactly
/// once per fresh database.
#[test]
fn schema_creation_is_idempotent_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("memory.db");

    let id = {
        let graph = MemoryGraph::open(&path).expect("first open");
        graph
            .create_board_item(&new_item("run-1", "survives reopen"))
            .expect("create")
            .id
    };

    let reopened = MemoryGraph::open(&path).expect("second open");
    assert_eq!(reopened.get_board_item(&id).expect("get").id, id);
}
