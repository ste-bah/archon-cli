// Unit coverage for the projection. The gate itself is exercised end to end in
// `workflow_live_v2_lifecycle_e2e_tests_e.rs`, through the composition root.

use std::sync::Arc;

use archon_memory::MemoryGraph;
use archon_memory::board::{BoardAccess, BoardItemKind, BoardStatus, NewBoardItem};

use super::*;

fn graph() -> Arc<MemoryGraph> {
    Arc::new(MemoryGraph::in_memory().expect("graph"))
}

fn raise(board: &MemoryGraph, run_id: &str, kind: BoardItemKind, title: &str) -> String {
    board
        .create_board_item(&NewBoardItem {
            id: None,
            run_id: run_id.to_string(),
            kind,
            title: title.to_string(),
            evidence: "src/command/workflow_board_drain.rs:1 -- fixture".to_string(),
            acceptance: "the projection carries it".to_string(),
            raised_by: "agent-a".to_string(),
        })
        .expect("create")
        .id
}

/// The port promises every item in the run, in any status, and nothing from any
/// other run: the gate reports what it inspected, and a projection that filtered
/// would make an empty board and a drained one read the same.
#[test]
fn the_port_returns_the_whole_run_partition_and_only_it() {
    let board = graph();
    let mine = raise(&board, "run-a", BoardItemKind::Issue, "open issue");
    let note = raise(&board, "run-a", BoardItemKind::Note, "a note");
    raise(&board, "run-b", BoardItemKind::Issue, "somebody else's");

    let port = MemoryBoardDrain::new(Arc::clone(&board) as Arc<dyn BoardAccess>);
    let items = port.drain_items_for_run("run-a").expect("drain read");

    assert_eq!(
        items
            .iter()
            .map(|item| (item.id.as_str(), item.kind, item.status))
            .collect::<Vec<_>>(),
        vec![
            (mine.as_str(), DrainItemKind::Issue, DrainStatus::Open),
            (note.as_str(), DrainItemKind::Note, DrainStatus::Open),
        ]
    );
    assert!(
        port.drain_items_for_run("run-absent")
            .expect("empty run")
            .is_empty()
    );
}

/// Every stored status has to arrive as itself. A projection that collapsed two
/// of them would either fail a drained run or accept an undrained one, and both
/// are silent.
#[test]
fn every_status_projects_onto_its_own_variant() {
    let board = graph();
    let expected = [
        (BoardStatus::Open, DrainStatus::Open),
        (BoardStatus::Claimed, DrainStatus::Claimed),
        (BoardStatus::InReview, DrainStatus::InReview),
        (BoardStatus::GapsRemain, DrainStatus::GapsRemain),
        (BoardStatus::Resolved, DrainStatus::Resolved),
        (BoardStatus::Promoted, DrainStatus::Promoted),
        (BoardStatus::Escalated, DrainStatus::Escalated),
    ];
    for (index, (stored, _)) in expected.iter().enumerate() {
        let id = raise(
            &board,
            "run-statuses",
            BoardItemKind::Issue,
            &format!("item {index}"),
        );
        if *stored != BoardStatus::Open {
            board
                .set_board_item_status(&id, BoardStatus::Open, *stored)
                .expect("transition");
        }
    }
    let declined = raise(&board, "run-statuses", BoardItemKind::Issue, "declined");
    board
        .decline_board_item(&declined, BoardStatus::Open, "already covered upstream")
        .expect("decline");

    let port = MemoryBoardDrain::new(Arc::clone(&board) as Arc<dyn BoardAccess>);
    let items = port
        .drain_items_for_run("run-statuses")
        .expect("drain read");

    let mut seen: Vec<DrainStatus> = items.iter().map(|item| item.status).collect();
    let mut want: Vec<DrainStatus> = expected.iter().map(|(_, drain)| *drain).collect();
    want.push(DrainStatus::Declined);
    seen.sort_by_key(|status| status.as_str());
    want.sort_by_key(|status| status.as_str());
    assert_eq!(seen, want);
}

/// The one field the gate judges that is not on the row. If the projection
/// dropped it, every declined item would fail the drain and the decline path
/// would be unusable in exactly the way it was before it could be stored.
#[test]
fn the_decline_reason_reaches_the_gate() {
    let board = graph();
    let declined = raise(&board, "run-reason", BoardItemKind::Issue, "not worth it");
    let resolved = raise(&board, "run-reason", BoardItemKind::Issue, "done");
    board
        .decline_board_item(
            &declined,
            BoardStatus::Open,
            "the caller already guards this",
        )
        .expect("decline");
    board
        .set_board_item_status(&resolved, BoardStatus::Open, BoardStatus::Resolved)
        .expect("resolve");

    let port = MemoryBoardDrain::new(Arc::clone(&board) as Arc<dyn BoardAccess>);
    let items = port.drain_items_for_run("run-reason").expect("drain read");

    let reasons: Vec<Option<&str>> = items
        .iter()
        .map(|item| item.decline_reason.as_deref())
        .collect();
    assert_eq!(
        reasons,
        vec![Some("the caller already guards this"), None],
        "only the declined item carries a reason, and it carries the one recorded"
    );
}

/// The half of #142 that makes the other half's regression loud. A process with
/// no board must hand the gate a port that REFUSES, because the driver reads a
/// missing port as "no board configured, pass" — and a run nobody could check is
/// not a run that was shown to be complete.
#[test]
fn a_process_without_a_board_refuses_the_drain_read_instead_of_answering_empty() {
    let port = UnreachableBoardDrain {
        reason: "no memory service is open in this process".to_string(),
    };

    let error = port
        .drain_items_for_run("wf-11111111")
        .expect_err("an absent board must not read as an empty one");

    let message = error.to_string();
    // The run id and the reason both travel into the `blocked-board-drain`
    // record: "this run left no gaps" and "nobody checked" are only
    // distinguishable afterwards if the refusal says which one it was.
    assert!(message.contains("wf-11111111"), "{message}");
    assert!(
        message.contains("no memory service is open in this process"),
        "{message}"
    );
}
