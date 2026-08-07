// The board drain gate, end to end, through the wiring a real run uses.
//
// Nothing here constructs a `LifecycleDriver` or calls `with_board_drain`. It
// installs a board the way session boot does, then runs
// `run_decomposed_lifecycle` — the same entry point `execute_generated_v2_run`
// calls — and asserts on the run's outcome. That is deliberate: a test that
// wired the gate itself would pass just as happily if the composition root
// never did, which is exactly the situation `acceptance.rs` was in.

use archon_memory::MemoryGraph;
use archon_memory::board::{BoardItemKind, BoardStatus, NewBoardItem};

// The board lives with the fixture, not here: since #142 every full-lifecycle
// run needs one installed or the drain gate refuses it, so installing it is part
// of building the fixture rather than something the drain tests do on their own.
use super::workflow_live_v2_lifecycle_e2e_tests_b::{
    full_lifecycle_fixture, installed_board, run_full_lifecycle,
};
use super::*;

fn raise(board: &MemoryGraph, run_id: &str, title: &str) -> String {
    board
        .create_board_item(&NewBoardItem {
            id: None,
            run_id: run_id.to_string(),
            kind: BoardItemKind::Issue,
            title: title.to_string(),
            evidence: "src/command/workflow_board_drain.rs:1 -- raised by the fixture".to_string(),
            acceptance: "the drain gate sees it".to_string(),
            raised_by: "agent-fixture".to_string(),
        })
        .expect("raise board item")
        .id
}

/// A run that would otherwise be accepted must not be, while it still owns an
/// open issue — and the refusal must name the item, because the point of the
/// board is that the handoff survives without another query.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_board_issue_blocks_a_run_that_would_otherwise_be_accepted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = full_lifecycle_fixture(temp.path());
    let board = installed_board();
    raise(&board, &fixture.run_id, "the retry path is never exercised");
    let v2_store = fixture.v2_store.clone();

    let summary = run_full_lifecycle(fixture.runner).await;

    assert_eq!(
        summary.status,
        WorkflowV2Status::NeedsReview,
        "an undrained board must stop acceptance, calls={:?}",
        summary
            .calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>()
    );
    let blocked = v2_store
        .load_call_record("blocked-board-drain")
        .expect("blocked record load")
        .expect("the drain gate must have reported");
    assert_eq!(blocked.status, WorkflowV2Status::NeedsReview);
    assert!(
        v2_store
            .load_call_record("final-acceptance-report")
            .expect("final record load")
            .is_none(),
        "the accepted report must not have been written behind the gate's back"
    );
}

/// The same run, with every issue ended properly, reaches acceptance. Without
/// this half the gate could be a constant `false` and the test above would not
/// notice.
///
/// The decline here is the whole reason gap 1 existed: before the storage layer
/// could hold a reason, this item could only have been declined into a state the
/// gate refuses, so the passing case was unreachable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fully_drained_board_lets_the_run_be_accepted() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = full_lifecycle_fixture(temp.path());
    let board = installed_board();
    let resolved = raise(&board, &fixture.run_id, "the retry path is never exercised");
    let declined = raise(&board, &fixture.run_id, "rename this module");
    let note = board
        .create_board_item(&NewBoardItem {
            id: None,
            run_id: fixture.run_id.clone(),
            kind: BoardItemKind::Note,
            title: "the fanout width is set by config here".to_string(),
            evidence: "src/command/workflow_live_v2_lifecycle.rs:1 -- context for next time"
                .to_string(),
            acceptance: "read by whoever next touches this".to_string(),
            raised_by: "agent-fixture".to_string(),
        })
        .expect("raise note")
        .id;

    board
        .set_board_item_status(&resolved, BoardStatus::Open, BoardStatus::Resolved)
        .expect("resolve");
    board
        .decline_board_item(
            &declined,
            BoardStatus::Open,
            "the name is load-bearing in the persisted metadata; renaming it breaks resume",
        )
        .expect("decline");
    let v2_store = fixture.v2_store.clone();

    let summary = run_full_lifecycle(fixture.runner).await;

    assert_eq!(
        summary.status,
        WorkflowV2Status::Accepted,
        "a drained board must not stop acceptance, calls={:?}",
        summary
            .calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        v2_store
            .load_call_record("blocked-board-drain")
            .expect("blocked record load")
            .is_none(),
        "a drained board must not report a drain failure"
    );
    // The note is still open and was still ignored: an issue is work that must
    // happen, a note is context that dies with the run, and a gate that drained
    // notes too would make writing one a reason to fail.
    assert_eq!(
        board.get_board_item(&note).expect("note").status,
        BoardStatus::Open
    );
    assert_eq!(
        board
            .get_board_item(&declined)
            .expect("declined")
            .decline_reason
            .as_deref(),
        Some("the name is load-bearing in the persisted metadata; renaming it breaks resume"),
        "the reason the gate accepted must still be on the record afterwards"
    );
}
