use archon_memory::board::{BoardAccess, BoardItemKind, BoardStatus, NewBoardItem};
use std::collections::BTreeMap;

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

/// A decline is only useful if its reason survives the transport. The drain
/// gate runs in the process that owns the workflow, which is not necessarily the
/// process that owns the writer, so a reason that existed only in-process would
/// look to the gate exactly like a decline with no reason at all -- and it fails
/// a run for that.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_decline_carries_its_reason_back_over_the_wire() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    let item = access
        .create_board_item(&new_item("run-decline", "declined remotely", "agent-a"))
        .expect("create");
    let reason = "the behaviour is deliberate; changing it would break the resume path";

    let declined = access
        .decline_board_item(&item.id, BoardStatus::Open, reason)
        .expect("remote decline");
    assert!(declined.applied);
    assert_eq!(declined.item.decline_reason.as_deref(), Some(reason));

    assert_eq!(
        graph
            .get_board_item(&item.id)
            .expect("direct get")
            .decline_reason
            .as_deref(),
        Some(reason),
        "a reason recorded over the wire must be readable by the process holding the graph"
    );
    assert_eq!(
        access.get_board_item(&item.id).expect("remote get"),
        graph.get_board_item(&item.id).expect("direct get"),
        "remote and direct reads of a declined item must agree in every field"
    );
    assert_eq!(
        access
            .list_board_items_by_run("run-decline", &[])
            .expect("remote list"),
        graph
            .list_board_items_by_run("run-decline", &[])
            .expect("direct list"),
        "the run-scoped read the drain gate uses must agree across the transport"
    );

    let history = access.board_item_history(&item.id).expect("remote history");
    assert_eq!(history, graph.board_item_history(&item.id).expect("direct"));
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].to_status, BoardStatus::Declined);
    assert_eq!(history[0].note, reason);

    handle.abort();
}

/// The requirement has to travel, not just the value. A remote caller must not
/// be able to reach `declined` through a shape with no reason in it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_remote_decline_without_a_reason_is_refused() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    let item = access
        .create_board_item(&new_item("run-no-reason", "unjustified", "agent-a"))
        .expect("create");

    let via_transition = access
        .set_board_item_status(&item.id, BoardStatus::Open, BoardStatus::Declined)
        .expect_err("the general transition must not reach declined over the wire either");
    assert!(
        via_transition.to_string().contains("decline_board_item"),
        "the refusal must survive the wire intact, got: {via_transition}"
    );

    let blank = access
        .decline_board_item(&item.id, BoardStatus::Open, "  ")
        .expect_err("a blank reason must be refused over the wire");
    assert!(blank.to_string().contains("reason"), "got: {blank}");

    assert_eq!(
        graph.get_board_item(&item.id).expect("direct get").status,
        BoardStatus::Open
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

/// Enumerating runs must give the same answer remotely as directly.
///
/// This is the read a caller makes when it holds no `run_id` at all, so a
/// direct-only implementation would not surface as an error anywhere the caller
/// could see: the remote client would get "unknown method" at best, and — had
/// the method been left off the dispatch table while the client method existed —
/// a board with no runs on it, reported as nothing to do.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_run_enumeration_matches_direct() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    assert!(
        access.list_board_runs().expect("remote runs").is_empty(),
        "an empty board must enumerate as no runs over the wire, not as an error"
    );
    assert_eq!(
        access.list_board_runs().expect("remote runs"),
        graph.list_board_runs().expect("direct runs")
    );

    let older = access
        .create_board_item(&new_item("run-older", "raised first", "agent-a"))
        .expect("create");
    access
        .create_board_item(&new_item("run-newer", "raised second", "agent-b"))
        .expect("create");
    access
        .create_board_item(&new_item("run-newer", "raised third", "agent-b"))
        .expect("create");
    // A transition rewrites `updated_at`, which is what the ordering is keyed
    // on, so touching the older run must move it back to the front.
    access
        .decline_board_item(&older.id, BoardStatus::Open, "already fixed upstream")
        .expect("decline");

    let remote_runs = access.list_board_runs().expect("remote runs");
    assert_eq!(
        remote_runs,
        graph.list_board_runs().expect("direct runs"),
        "remote and direct run enumeration must agree, order and counts included"
    );

    let ids: Vec<&str> = remote_runs.iter().map(|run| run.run_id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["run-older", "run-newer"],
        "most recently touched first"
    );
    assert_eq!(remote_runs[0].total, 1);
    assert_eq!(
        remote_runs[0].counts,
        BTreeMap::from([("declined".to_string(), 1)]),
        "a status with no items must be absent rather than present as zero"
    );
    assert_eq!(remote_runs[1].total, 2);
    assert_eq!(
        remote_runs[1].counts,
        BTreeMap::from([("open".to_string(), 2)])
    );

    // The run list is only useful if what it names can then be opened, so the
    // handoff from enumeration to the run-scoped read is part of the contract.
    let items = access
        .list_board_items_by_run(&remote_runs[0].run_id, &[])
        .expect("remote items");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].decline_reason.as_deref(),
        Some("already fixed upstream")
    );

    handle.abort();
}

/// The run feed must cross the wire, for the reason #128 exists.
///
/// A memory operation implemented directly but left off the dispatch table does
/// not fail loudly: the client method still compiles, the call still returns,
/// and the second process reads the empty result as "this run has no history".
/// The dashboard is almost always that second process -- the TUI holds the
/// writer -- so a direct-only activity read would be an activity feed that is
/// blank exactly when it matters.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_run_activity_matches_direct() {
    let (_dir, port_file) = temp_port_file();
    let (port, graph, handle) = start_test_server(port_file).await;
    let access = remote(port).await;

    assert!(
        access
            .board_run_activity("run-feed")
            .expect("remote activity")
            .is_empty(),
        "a run with no history must answer as an empty feed over the wire"
    );

    let item = access
        .create_board_item(&new_item("run-feed", "raised remotely", "agent-a"))
        .expect("create");
    access
        .create_board_item(&new_item("run-other", "different run", "agent-b"))
        .expect("create");
    access
        .set_board_item_status(&item.id, BoardStatus::Open, BoardStatus::Claimed)
        .expect("claim transition");
    access
        .decline_board_item(&item.id, BoardStatus::Claimed, "already handled upstream")
        .expect("decline");

    let remote_feed = access.board_run_activity("run-feed").expect("remote feed");
    assert_eq!(
        remote_feed,
        graph.board_run_activity("run-feed").expect("direct feed"),
        "remote and direct feeds must agree, order and every column included"
    );
    assert_eq!(remote_feed.len(), 2, "feed: {remote_feed:?}");
    assert_eq!(
        remote_feed[0].to_status,
        BoardStatus::Declined,
        "newest first must survive serialisation"
    );
    assert_eq!(remote_feed[0].note, "already handled upstream");

    handle.abort();
}

// ═══════════════════════════════════════════════════════════════
