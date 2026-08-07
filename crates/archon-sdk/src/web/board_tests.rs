//! Board endpoint tests.
//!
//! Every one of these goes through a real memory database on disk rather than a
//! fake `BoardAccess`. The thing most likely to be wrong here is not the
//! projection but whether the handler reaches the store at all — that is the
//! shape of the bug this endpoint exists to avoid repeating (issue #128) — and a
//! fake board would answer that question by construction.

use std::path::Path;

use archon_memory::MemoryGraph;
use archon_memory::board::{BoardItemKind, BoardStatus, NewBoardItem};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

use super::super::api::{EffectivePolicySummary, WebApiState};
use super::super::server::build_app;
use super::*;
use crate::web::{WebConfig, WebLiveManager, WebRuntimeHandles, agents, ingest};

fn config() -> WebConfig {
    WebConfig {
        open_browser: false,
        ..WebConfig::default()
    }
}

fn state_for(db: &Path, token: Option<String>) -> AppState {
    let config = config();
    let auth_required = token.is_some();
    let paths = WebRuntimePaths::from_overrides(Some(&db.to_string_lossy()), None);
    AppState {
        token,
        api: WebApiState::from_server_config(
            &config,
            auth_required,
            EffectivePolicySummary::default_safe(),
            paths.clone(),
        ),
        live: WebLiveManager::new(16),
        paths,
        chat_backend: None,
        ingest_jobs: ingest::new_job_store(),
        handles: WebRuntimeHandles::default(),
        agents: agents::WebAgentObserver::new(),
        board: WebBoardStore::new(),
        attached: false,
    }
}

fn raise(graph: &MemoryGraph, run_id: &str, title: &str) -> String {
    graph
        .create_board_item(&NewBoardItem {
            id: None,
            run_id: run_id.to_string(),
            kind: BoardItemKind::Issue,
            title: title.to_string(),
            evidence: "crates/archon-sdk/src/web/board.rs:1 -- observed on read".to_string(),
            acceptance: "the endpoint returns the row".to_string(),
            raised_by: "agent-a".to_string(),
        })
        .expect("raise")
        .id
}

/// Seed a throwaway database and close it before the server opens its own.
///
/// The handler resolves the board by path, so the two would otherwise be two
/// CozoDB instances over one sqlite file at the same time — which is the
/// arrangement the memory server exists to prevent.
fn seeded(dir: &Path, seed: impl FnOnce(&MemoryGraph)) -> std::path::PathBuf {
    let db = dir.join("memory.db");
    let graph = MemoryGraph::open(&db).expect("open seed graph");
    seed(&graph);
    drop(graph);
    db
}

async fn get(state: AppState, uri: &str) -> (StatusCode, String) {
    let response = build_app(&config(), state)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("request");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// A board with no database behind it must say so rather than report an empty
/// board: "nothing raised yet" and "no store to read" are different facts, and
/// only one of them is worth investigating.
#[tokio::test]
async fn a_missing_database_is_reported_as_an_unavailable_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = state_for(&dir.path().join("absent.db"), None);

    let (status, body) = get(state, "/api/board/runs").await;
    assert_eq!(status, StatusCode::OK);
    let runs: WebBoardRunList = serde_json::from_str(&body).expect("runs");
    assert!(!runs.store_available, "{body}");
    assert!(runs.runs.is_empty());
}

/// A database that exists but holds nothing is the empty board, and it must
/// come back as available-and-empty.
#[tokio::test]
async fn an_empty_board_is_available_and_has_no_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |_| {});

    let (status, body) = get(state_for(&db, None), "/api/board/runs").await;
    assert_eq!(status, StatusCode::OK);
    let runs: WebBoardRunList = serde_json::from_str(&body).expect("runs");
    assert!(runs.store_available, "{body}");
    assert!(runs.runs.is_empty(), "{body}");
}

#[tokio::test]
async fn runs_carry_per_status_counts_most_recently_touched_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |graph| {
        let first = raise(graph, "run-early", "raised first");
        raise(graph, "run-late", "raised second");
        raise(graph, "run-late", "raised third");
        // Moves `run-early` back to the front: the ordering is on updated_at.
        graph.claim_board_item(&first, "agent-b").expect("claim");
        graph
            .set_board_item_status(&first, BoardStatus::Claimed, BoardStatus::InReview)
            .expect("to review");
    });

    let (status, body) = get(state_for(&db, None), "/api/board/runs").await;
    assert_eq!(status, StatusCode::OK);
    let runs: WebBoardRunList = serde_json::from_str(&body).expect("runs");
    assert!(runs.store_available);

    let ids: Vec<&str> = runs.runs.iter().map(|run| run.run_id.as_str()).collect();
    assert_eq!(ids, vec!["run-early", "run-late"], "{body}");
    assert_eq!(runs.runs[0].total, 1);
    assert_eq!(
        runs.runs[0].counts,
        vec![WebBoardStatusCount {
            status: "in_review".to_string(),
            count: 1,
        }]
    );
    assert_eq!(runs.runs[1].total, 2);
    assert_eq!(runs.runs[1].counts[0].status, "open");
    assert_eq!(runs.runs[1].counts[0].count, 2);
}

#[tokio::test]
async fn a_run_lists_its_items_and_honours_a_status_filter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |graph| {
        let claimed = raise(graph, "run-mixed", "being worked");
        raise(graph, "run-mixed", "still open");
        graph.claim_board_item(&claimed, "agent-b").expect("claim");
    });

    let (status, body) = get(state_for(&db, None), "/api/board/runs/run-mixed/items").await;
    assert_eq!(status, StatusCode::OK);
    let items: WebBoardItems = serde_json::from_str(&body).expect("items");
    assert!(items.store_available);
    assert!(items.statuses.is_empty(), "no filter was requested");
    assert_eq!(items.items.len(), 2, "{body}");
    let worked = &items.items[0];
    assert_eq!(worked.title, "being worked");
    assert_eq!(worked.kind, "issue");
    assert_eq!(worked.status, "claimed");
    assert_eq!(worked.claimed_by.as_deref(), Some("agent-b"));
    assert_eq!(worked.raised_by, "agent-a");
    assert_eq!(worked.round, 0);
    assert!(worked.evidence.contains("board.rs:1"));
    assert!(worked.decline_reason.is_none());

    let (status, body) = get(
        state_for(&db, None),
        "/api/board/runs/run-mixed/items?status=claimed",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let filtered: WebBoardItems = serde_json::from_str(&body).expect("filtered");
    assert_eq!(filtered.statuses, vec!["claimed".to_string()]);
    assert_eq!(filtered.items.len(), 1, "{body}");
    assert_eq!(filtered.items[0].title, "being worked");
}

/// A misspelt status must be refused. Silently ignoring it would answer with
/// the whole board while the caller believed it had filtered, or with nothing
/// while the caller believed the status was empty.
#[tokio::test]
async fn an_unknown_status_filter_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |graph| {
        raise(graph, "run-mixed", "still open");
    });

    let (status, body) = get(
        state_for(&db, None),
        "/api/board/runs/run-mixed/items?status=open,in-review",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("in-review"), "{body}");
}

/// The decline reason is derived from the transition history, not stored on the
/// row, so the projection is where it can quietly go missing — and a declined
/// item without its justification is exactly what the drain gate fails a run for.
#[tokio::test]
async fn a_declined_item_carries_its_reason_and_its_transition_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reason = "the behaviour is deliberate; changing it would break the resume path";
    let mut declined_id = String::new();
    let db = seeded(dir.path(), |graph| {
        let id = raise(graph, "run-declined", "not a bug");
        graph
            .decline_board_item(&id, BoardStatus::Open, reason)
            .expect("decline");
        declined_id = id;
    });

    let (status, body) = get(
        state_for(&db, None),
        "/api/board/runs/run-declined/items?status=declined",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items: WebBoardItems = serde_json::from_str(&body).expect("items");
    assert_eq!(items.items.len(), 1, "{body}");
    assert_eq!(items.items[0].status, "declined");
    assert_eq!(items.items[0].decline_reason.as_deref(), Some(reason));

    let (status, body) = get(
        state_for(&db, None),
        &format!("/api/board/items/{declined_id}/history"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let history: WebBoardHistory = serde_json::from_str(&body).expect("history");
    assert!(history.store_available);
    assert_eq!(history.item_id, declined_id);
    assert_eq!(history.events.len(), 1, "{body}");
    assert_eq!(history.events[0].seq, 0);
    assert_eq!(history.events[0].from_status, "open");
    assert_eq!(history.events[0].to_status, "declined");
    assert_eq!(history.events[0].note, reason);
    assert!(
        history.events[0].actor.is_none(),
        "nobody held the item, and that must not become an agent named nothing"
    );
}

/// An item that only ever sat open has no transitions. That is an empty history
/// rather than a missing one — claims and releases are deliberately not recorded.
#[tokio::test]
async fn an_item_with_no_transitions_has_an_empty_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut id = String::new();
    let db = seeded(dir.path(), |graph| {
        id = raise(graph, "run-quiet", "never moved");
    });

    let (status, body) = get(
        state_for(&db, None),
        &format!("/api/board/items/{id}/history"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let history: WebBoardHistory = serde_json::from_str(&body).expect("history");
    assert!(history.events.is_empty(), "{body}");
}

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
            alone.elected().expect("elected").as_ref(),
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
            alongside.elected().expect("elected").as_ref(),
            archon_memory::access::MemoryAccess::Remote(_)
        ),
        "a second process must reach the board over the socket, not open the database again"
    );

    drop(host);
}

/// The board is a read of everything every agent in the session raised, so it
/// takes the same bearer token every other endpoint does.
#[tokio::test]
async fn board_endpoints_require_the_bearer_token_when_one_is_configured() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = seeded(dir.path(), |graph| {
        raise(graph, "run-guarded", "protected");
    });
    let token = "board-token";

    for uri in [
        "/api/board/runs",
        "/api/board/runs/run-guarded/items",
        "/api/board/items/whatever/history",
    ] {
        let (status, _) = get(state_for(&db, Some(token.to_string())), uri).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");

        let response = build_app(&config(), state_for(&db, Some(token.to_string())))
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
    }
}
