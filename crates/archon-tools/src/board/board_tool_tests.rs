use std::path::PathBuf;
use std::sync::Arc;

use archon_memory::board::{BoardAccess, BoardStatus};
use archon_memory::graph::MemoryGraph;

use super::tools::{BoardListTool, BoardRaiseTool};
use super::tools_lifecycle::{BoardClaimTool, BoardResolveTool};
use super::{TOP_LEVEL_AGENT, run_id_for_session};
use crate::tool::{Tool, ToolContext};

fn board() -> Arc<dyn BoardAccess> {
    Arc::new(MemoryGraph::in_memory().expect("in-memory graph"))
}

fn ctx(session_id: &str, subagent_id: Option<&str>) -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("."),
        session_id: session_id.to_string(),
        subagent_id: subagent_id.map(str::to_string),
        mode: crate::tool::AgentMode::Normal,
        ..Default::default()
    }
}

fn raise_input(title: &str) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "evidence": "crates/archon-tools/src/board.rs:1 -- observed while reading",
        "acceptance": "the item is reachable from BoardList",
    })
}

// ── run id extraction ──────────────────────────────────────────

/// A workflow stage session carries its run in a prefix; an interactive one is
/// its own run. Both shapes have to land on the same partition their siblings
/// use, or a board is silently split.
#[test]
fn run_id_comes_from_the_prefix_of_a_workflow_session() {
    assert_eq!(
        run_id_for_session("run-42-stage-implement-attempt-1"),
        "run-42"
    );
    assert_eq!(
        run_id_for_session("2026-08-06T12-00-00-stage-fanout-3-attempt-2"),
        "2026-08-06T12-00-00"
    );
}

#[test]
fn run_id_of_a_plain_interactive_session_is_the_session_itself() {
    assert_eq!(run_id_for_session("sess-abc123"), "sess-abc123");
    assert_eq!(run_id_for_session(""), "");
}

/// A stage id may itself contain `-stage-`; splitting on the last occurrence
/// would fold half the stage name into the run and split the run's board.
#[test]
fn run_id_splits_on_the_first_stage_marker() {
    assert_eq!(
        run_id_for_session("run-7-stage-rebuild-stage-two-attempt-1"),
        "run-7"
    );
}

// ── attribution ────────────────────────────────────────────────

/// The point of `ToolContext::subagent_id`: the stored row must name the exact
/// agent that wrote it, not merely have something in the field.
#[tokio::test]
async fn a_raise_from_a_subagent_is_attributed_to_that_subagent() {
    let board = board();
    let tool = BoardRaiseTool::with_access(Arc::clone(&board));
    let subagent = "1b7f4c2a-0000-4000-8000-00000000abcd";

    let result = tool
        .execute(raise_input("attributed"), &ctx("sess-1", Some(subagent)))
        .await;
    assert!(!result.is_error, "raise failed: {}", result.content);

    let items = board.list_board_items_by_run("sess-1", &[]).expect("list");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].raised_by, subagent,
        "the stored raiser must be the calling subagent's id verbatim"
    );
}

/// `None` is the top-level agent making the call directly. That is a real
/// answer and must be recorded, not rejected.
#[tokio::test]
async fn a_raise_from_the_top_level_agent_records_no_subagent() {
    let board = board();
    let tool = BoardRaiseTool::with_access(Arc::clone(&board));

    let result = tool
        .execute(raise_input("top level"), &ctx("sess-2", None))
        .await;
    assert!(
        !result.is_error,
        "a top-level call must not error: {}",
        result.content
    );

    let items = board.list_board_items_by_run("sess-2", &[]).expect("list");
    assert_eq!(items[0].raised_by, TOP_LEVEL_AGENT);
}

#[tokio::test]
async fn a_claim_from_a_subagent_is_attributed_to_that_subagent() {
    let board = board();
    let raise = BoardRaiseTool::with_access(Arc::clone(&board));
    let claim = BoardClaimTool::with_access(Arc::clone(&board));
    let subagent = "1b7f4c2a-0000-4000-8000-00000000bcde";

    raise
        .execute(raise_input("claim me"), &ctx("sess-3", None))
        .await;
    let id = board.list_board_items_by_run("sess-3", &[]).expect("list")[0]
        .id
        .clone();

    let result = claim
        .execute(
            serde_json::json!({ "id": id }),
            &ctx("sess-3", Some(subagent)),
        )
        .await;
    assert!(!result.is_error, "claim failed: {}", result.content);

    let stored = board.get_board_item(&id).expect("get");
    assert_eq!(stored.claimed_by.as_deref(), Some(subagent));
    assert_eq!(stored.status, BoardStatus::Claimed);
}

// ── contested claim ────────────────────────────────────────────

/// A refused claim must say who holds the item. Being told only "no" leaves the
/// loser unable to tell a live owner from a bug.
#[tokio::test]
async fn claiming_a_held_item_reports_failure_and_names_the_holder() {
    let board = board();
    let raise = BoardRaiseTool::with_access(Arc::clone(&board));
    let claim = BoardClaimTool::with_access(Arc::clone(&board));
    // A holder the sweep will treat as live, so the refusal is about the claim
    // and not about a lease that got collected mid-test.
    let holder = TOP_LEVEL_AGENT;
    let loser = "1b7f4c2a-0000-4000-8000-00000000cdef";

    raise
        .execute(raise_input("contested"), &ctx("sess-4", None))
        .await;
    let id = board.list_board_items_by_run("sess-4", &[]).expect("list")[0]
        .id
        .clone();

    let first = claim
        .execute(serde_json::json!({ "id": &id }), &ctx("sess-4", None))
        .await;
    assert!(!first.is_error, "first claim: {}", first.content);

    let second = claim
        .execute(
            serde_json::json!({ "id": &id }),
            &ctx("sess-4", Some(loser)),
        )
        .await;
    assert!(
        second.is_error,
        "a losing claim must not read as success: {}",
        second.content
    );
    assert!(
        second.content.contains(holder),
        "the refusal must name the holder, got: {}",
        second.content
    );

    assert_eq!(
        board
            .get_board_item(&id)
            .expect("get")
            .claimed_by
            .as_deref(),
        Some(holder),
        "a refused claim must not disturb the holder"
    );
}

// ── evidence ───────────────────────────────────────────────────

#[tokio::test]
async fn raising_without_evidence_is_a_clear_tool_error() {
    let board = board();
    let tool = BoardRaiseTool::with_access(Arc::clone(&board));

    for blank in [serde_json::json!(""), serde_json::json!("   ")] {
        let result = tool
            .execute(
                serde_json::json!({ "title": "no evidence", "evidence": blank }),
                &ctx("sess-5", None),
            )
            .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("evidence"),
            "the error must name the missing field, got: {}",
            result.content
        );
    }
    assert!(
        board
            .list_board_items_by_run("sess-5", &[])
            .expect("list")
            .is_empty()
    );
}

// ── list and resolve ───────────────────────────────────────────

#[tokio::test]
async fn list_is_scoped_to_the_run_and_filtered_by_status() {
    let board = board();
    let raise = BoardRaiseTool::with_access(Arc::clone(&board));
    let list = BoardListTool::with_access(Arc::clone(&board));

    raise
        .execute(raise_input("mine"), &ctx("run-9-stage-a-attempt-1", None))
        .await;
    raise
        .execute(raise_input("someone else's"), &ctx("run-other", None))
        .await;

    let result = list
        .execute(
            serde_json::json!({ "status": ["open"] }),
            &ctx("run-9-stage-b-attempt-2", None),
        )
        .await;
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("mine"));
    assert!(
        !result.content.contains("someone else's"),
        "a sibling stage must see its own run only: {}",
        result.content
    );
}

#[tokio::test]
async fn resolve_requires_a_reason_and_moves_the_status() {
    let board = board();
    let raise = BoardRaiseTool::with_access(Arc::clone(&board));
    let resolve = BoardResolveTool::with_access(Arc::clone(&board));

    raise
        .execute(raise_input("close me"), &ctx("sess-6", None))
        .await;
    let id = board.list_board_items_by_run("sess-6", &[]).expect("list")[0]
        .id
        .clone();

    let missing = resolve
        .execute(
            serde_json::json!({ "id": &id, "outcome": "declined" }),
            &ctx("sess-6", None),
        )
        .await;
    assert!(missing.is_error);
    assert!(missing.content.contains("reason"));

    let closed = resolve
        .execute(
            serde_json::json!({
                "id": &id,
                "outcome": "declined",
                "reason": "superseded by the storage-layer change in board/crud.rs",
            }),
            &ctx("sess-6", None),
        )
        .await;
    assert!(!closed.is_error, "{}", closed.content);
    assert_eq!(
        board.get_board_item(&id).expect("get").status,
        BoardStatus::Declined
    );
}

#[tokio::test]
async fn a_tool_with_no_board_handle_says_so() {
    // `BoardHandle::Global` with nothing installed. Test binaries never call
    // `install_board_access`, so this is the real unconfigured path.
    let result = BoardRaiseTool::new()
        .execute(raise_input("nowhere to go"), &ctx("sess-7", None))
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("task board is unavailable"));
}
