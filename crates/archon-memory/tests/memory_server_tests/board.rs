use archon_memory::board::{BoardAccess, BoardItemKind, BoardStatus, NewBoardItem};

use super::support::*;
use super::*;

// Task board over the wire
// ═══════════════════════════════════════════════════════════════

fn new_item(run_id: &str, title: &str, raised_by: &str) -> NewBoardItem {
    NewBoardItem {
        id: None,
        run_id: run_id.to_string(),
        kind: BoardItemKind::Issue,
        title: title.to_string(),
        evidence: "crates/archon-memory/src/board.rs:1 -- observed on read".to_string(),
        acceptance: "the remote answer equals the direct one".to_string(),
        raised_by: raised_by.to_string(),
    }
}

async fn remote(port: u16) -> MemoryAccess {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    // `MemoryAccess::Remote` holds nothing but the socket, so any answer it
    // gives can only have come back over TCP.
    MemoryAccess::Remote(MemoryClient::connect(addr).await.expect("connect"))
}

/// Every board operation must give the same answer remotely as directly.
///
/// A second Archon process reaches the graph only through this socket, because
/// CozoDB admits one writer. A board that worked in-process and not over the
/// wire would be a board no subagent in another process could hand work to --
/// and the failure would look like an empty board rather than an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_board_round_trip_matches_direct() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    let created = access
        .create_board_item(&new_item("run-wire", "raised remotely", "agent-remote"))
        .expect("remote create");
    assert_eq!(
        graph.get_board_item(&created.id).expect("direct get"),
        created,
        "the row the server stored must be the row the client was told about"
    );

    let direct_raised = graph
        .create_board_item(&new_item("run-wire", "raised directly", "agent-direct"))
        .expect("direct create");
    assert_eq!(
        access
            .get_board_item(&direct_raised.id)
            .expect("remote get"),
        direct_raised,
        "a directly-raised item must read back identically over the wire"
    );

    let claim = access
        .claim_board_item(&created.id, "agent-remote")
        .expect("remote claim");
    assert!(claim.applied);
    assert_eq!(claim.item.claimed_by.as_deref(), Some("agent-remote"));
    assert_eq!(
        graph.get_board_item(&created.id).expect("direct get"),
        claim.item,
        "a claim taken over the wire must be visible to the process holding the graph"
    );

    assert_eq!(
        access
            .list_board_items_by_run("run-wire", &[])
            .expect("remote list"),
        graph
            .list_board_items_by_run("run-wire", &[])
            .expect("direct list"),
        "remote and direct listings must agree, order included"
    );
    assert_eq!(
        access
            .list_board_items_by_run("run-wire", &[BoardStatus::Open])
            .expect("remote filtered list"),
        vec![direct_raised],
        "a status filter must survive the wire"
    );

    handle.abort();
}

/// The compare-and-set has to hold across the transport, not just inside one
/// process. If it did not, two agents in two processes -- the normal case, since
/// every session after the first is a client -- would both be told they claimed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_claim_taken_directly_is_refused_over_the_wire_and_the_reverse() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    let taken_directly = graph
        .create_board_item(&new_item("run-cas", "direct wins", "agent-a"))
        .expect("create");
    assert!(
        graph
            .claim_board_item(&taken_directly.id, "agent-direct")
            .expect("direct claim")
            .applied
    );
    let refused = access
        .claim_board_item(&taken_directly.id, "agent-remote")
        .expect("remote claim");
    assert!(
        !refused.applied,
        "the remote caller must be told it lost, not silently handed the item"
    );
    assert_eq!(refused.item.claimed_by.as_deref(), Some("agent-direct"));

    let taken_remotely = graph
        .create_board_item(&new_item("run-cas", "remote wins", "agent-b"))
        .expect("create");
    assert!(
        access
            .claim_board_item(&taken_remotely.id, "agent-remote")
            .expect("remote claim")
            .applied
    );
    assert!(
        !graph
            .claim_board_item(&taken_remotely.id, "agent-direct")
            .expect("direct claim")
            .applied
    );

    handle.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_status_transition_is_refused_from_the_wrong_prior_status() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    let item = access
        .create_board_item(&new_item("run-status", "verdict", "agent-a"))
        .expect("create");
    access.claim_board_item(&item.id, "agent-a").expect("claim");
    assert!(
        access
            .set_board_item_status(&item.id, BoardStatus::Claimed, BoardStatus::InReview)
            .expect("to review")
            .applied
    );

    let stale = access
        .set_board_item_status(&item.id, BoardStatus::Claimed, BoardStatus::Escalated)
        .expect("stale transition");
    assert!(
        !stale.applied,
        "a stale prior status must be refused over the wire too"
    );
    assert_eq!(stale.item.status, BoardStatus::InReview);
    assert_eq!(
        graph.get_board_item(&item.id).expect("direct get").status,
        BoardStatus::InReview
    );

    handle.abort();
}

/// The evidence rule is enforced at the store, so it must reach a remote caller
/// as a refusal rather than an empty success.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_create_without_evidence_is_refused() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    let mut item = new_item("run-empty", "no evidence", "agent-a");
    item.evidence = "   ".to_string();
    let error = access
        .create_board_item(&item)
        .expect_err("an item without evidence must be refused across the wire");
    assert!(
        error.to_string().contains("evidence"),
        "the refusal must survive the wire intact, got: {error}"
    );
    assert!(
        graph
            .list_board_items_by_run("run-empty", &[])
            .expect("direct list")
            .is_empty()
    );

    handle.abort();
}

// ═══════════════════════════════════════════════════════════════
