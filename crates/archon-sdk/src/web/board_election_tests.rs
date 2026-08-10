//! Tests for the store election in `board_store.rs`.
//!
//! Split from `board_tests.rs` by subject rather than by size: everything there
//! asks what the endpoints answer, and everything here asks which reader they
//! answered from. These are also the slow ones — each stands up a real memory
//! server on a multi-threaded runtime — and they are the only tests in the file
//! that assert on the elected arm rather than on a response body.

use std::sync::Arc;

use axum::http::StatusCode;

use crate::web::WebRuntimePaths;
use crate::web::board::{WebBoardItems, WebBoardRunList, WebBoardStore};

use super::{get, raise, seeded, state_for};

/// The attached case: a host session already owns the writer.
///
/// This is the arrangement that was broken. CozoDB admits one writer, so when a
/// TUI holds the memory graph on this file, the web server must reach it as a
/// client rather than opening the database a second time. `open_memory_with_db_path`
/// is what decides that, by reading the `memory.port` the running server wrote —
/// and going around it with a direct `MemoryGraph::open` is a second writer
/// against a locked database (issue #134).
///
/// Multi-threaded on purpose: the host's memory server has to be able to serve
/// the request while this test is blocked waiting for the response.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_board_is_reachable_while_another_process_holds_the_writer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |graph| {
        raise(graph, "run-attached", "raised by the session");
    });

    // Stand in for the host session: this takes `memory.lock`, opens the graph,
    // and starts the memory server that writes `memory.port`.
    let host = archon_memory::open_memory_with_db_path(dir.path(), &db)
        .await
        .expect("host session opens memory");
    assert!(
        dir.path().join("memory.port").exists(),
        "the host must be the elected server for this test to mean anything"
    );

    let (status, body) = get(state_for(&db, None), "/api/board/runs").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let runs: WebBoardRunList = serde_json::from_str(&body).expect("runs");
    assert!(runs.store_available, "{body}");
    assert_eq!(runs.runs.len(), 1, "{body}");
    assert_eq!(runs.runs[0].run_id, "run-attached");

    let (status, body) = get(state_for(&db, None), "/api/board/runs/run-attached/items").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items: WebBoardItems = serde_json::from_str(&body).expect("items");
    assert_eq!(items.items.len(), 1, "{body}");
    assert_eq!(items.items[0].title, "raised by the session");

    drop(host);
}

/// The regression guard for issue #134, and it has to assert the elected mode
/// rather than the answer.
///
/// A raw second `MemoryGraph::open` of a database another process holds does not
/// hang and does not error — measured on this branch, it succeeds in 206ms from a
/// genuinely separate OS process and reads correct rows. So every behavioural
/// assertion in this file passes just as well with the bug in place. What the bug
/// actually does is create a second CozoDB writer on a file whose single-writer
/// invariant nothing else enforces, and the only thing that distinguishes it from
/// the fix is which arm the election returned.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_store_connects_as_a_client_when_a_server_already_holds_the_writer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |graph| {
        raise(graph, "run-elected", "raised by the session");
    });
    let paths = WebRuntimePaths::from_overrides(Some(&db.to_string_lossy()), None);

    // With nobody holding it, the web server is entitled to be the writer.
    let alone = WebBoardStore::new();
    alone.resolve(&paths).await.expect("resolve").expect("some");
    assert!(
        matches!(
            alone.elected().await.expect("elected").as_ref(),
            archon_memory::access::MemoryAccess::Direct { .. }
        ),
        "with no server running the election should have opened the graph directly"
    );
    drop(alone);

    // Now a host owns the writer and is serving on `memory.port`.
    let host = archon_memory::open_memory_with_db_path(dir.path(), &db)
        .await
        .expect("host holds the writer");
    let alongside = WebBoardStore::new();
    alongside
        .resolve(&paths)
        .await
        .expect("resolve")
        .expect("some");
    assert!(
        matches!(
            alongside.elected().await.expect("elected").as_ref(),
            archon_memory::access::MemoryAccess::Remote(_)
        ),
        "a second process must reach the board over the socket, not open the database again"
    );

    drop(host);
}

/// A cached election survives a poll but not a failure.
///
/// The bug this pins: the elected arm is `Remote` whenever another process owns
/// the writer, and that server dies with its process. A write-once cache then
/// hands back the closed socket for the life of the web server, so every board
/// request returns 500 even once a healthy server is listening again — which is
/// exactly what was observed live, an 8-hour-old web process holding a CLOSED
/// socket to a server that had long exited.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dead_election_is_dropped_so_the_next_request_re_elects() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |graph| {
        raise(graph, "run-recovered", "raised before the server died");
    });
    let paths = WebRuntimePaths::from_overrides(Some(&db.to_string_lossy()), None);

    let store = WebBoardStore::new();
    store.resolve(&paths).await.expect("resolve").expect("some");
    let first = store.elected().await.expect("elected");

    // A second resolve reuses the election rather than re-running it: that is
    // the caching this fix must not have thrown away.
    store.resolve(&paths).await.expect("resolve").expect("some");
    assert!(
        Arc::ptr_eq(&first, &store.elected().await.expect("elected")),
        "a healthy election must be reused, not re-run on every poll"
    );

    // A read failed, so the handle is suspect and gets dropped.
    store.invalidate().await;
    assert!(
        store.elected().await.is_none(),
        "invalidate must clear the slot, or the next request reuses the dead handle"
    );

    // The next request elects again and the board is readable.
    let board = store.resolve(&paths).await.expect("resolve").expect("some");
    let runs = board
        .list_board_runs()
        .expect("the re-elected handle reads");
    assert!(
        runs.iter().any(|run| run.run_id == "run-recovered"),
        "re-electing must produce a handle that can actually read the board"
    );
}
