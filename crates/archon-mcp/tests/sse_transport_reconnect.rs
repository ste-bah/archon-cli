//! TASK-202 MCP-SSE-HARDEN-RETRY — reconnect + Last-Event-ID replay.
//!
//! Proves the hardened SSE pump:
//!   1. Automatically reconnects after the server drops the stream.
//!   2. Sends `Last-Event-ID: <last-seen>` on the reconnect GET.
//!   3. Continues delivering frames to the caller transparently.
//!   4. Bounded retries — persistent-down server eventually gives up.
//!
//! The mock server accepts 2 SSE GETs:
//!   * 1st: emits 3 framed events with ids 1/2/3, then ENDS the stream.
//!   * 2nd: reads `Last-Event-ID` from request headers, records it,
//!     emits events 4/5, keeps the stream open.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, Sse};
use axum::routing::get;
use axum::Router;
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use archon_mcp::sse_reconnect::{ReconnectConfig, pump_sse_stream_with_reconnect};

// ---------------------------------------------------------------------------
// Mock server state
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockState {
    /// Number of GETs received (first = 0, second = 1, ...).
    get_count: Arc<AtomicU32>,
    /// Last-Event-ID header captured on each incoming GET (one entry per GET).
    last_event_ids: Arc<Mutex<Vec<String>>>,
}

type BoxedEventStream =
    Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send + 'static>>;

async fn sse_handler(State(state): State<MockState>, headers: HeaderMap) -> Sse<BoxedEventStream> {
    let get_n = state.get_count.fetch_add(1, Ordering::SeqCst);

    let captured = headers
        .get("last-event-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    state.last_event_ids.lock().await.push(captured);

    let events: Vec<Event> = if get_n == 0 {
        vec![
            Event::default().id("1").data("one"),
            Event::default().id("2").data("two"),
            Event::default().id("3").data("three"),
        ]
    } else {
        vec![
            Event::default().id("4").data("four"),
            Event::default().id("5").data("five"),
        ]
    };

    let event_stream = stream::iter(events.into_iter().map(Ok::<_, Infallible>));
    let combined: BoxedEventStream = if get_n == 0 {
        // First GET: stream ends naturally after 3 events -> server closes.
        Box::pin(event_stream)
    } else {
        // Second+ GETs: chain with stream::pending so the connection stays open.
        Box::pin(event_stream.chain(stream::pending()))
    };

    Sse::new(combined)
}

async fn spawn_mock() -> (SocketAddr, tokio::task::JoinHandle<()>, MockState) {
    let state = MockState::default();
    let app = Router::new()
        .route("/sse", get(sse_handler))
        .with_state(state.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, server, state)
}

// ---------------------------------------------------------------------------
// Test 1: Reconnect with Last-Event-ID
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_auto_reconnects_and_sends_last_event_id() {
    use tokio::sync::mpsc;

    let (addr, server, state) = spawn_mock().await;
    let url = format!("http://{addr}/sse");

    let (tx, mut rx) = mpsc::channel::<archon_mcp::sse_transport::SseFrame>(64);
    let client = reqwest::Client::builder()
        .build()
        .expect("build reqwest client");

    let config = ReconnectConfig {
        default_retry_ms: 50, // short to keep test fast
        max_retries: 5,
        jitter_ratio: 0.0,    // deterministic for test
    };

    tokio::spawn(pump_sse_stream_with_reconnect(
        client,
        url,
        std::collections::HashMap::new(),
        tx,
        config,
    ));

    // Collect 5 frames total: 3 from first GET, 2 from reconnect.
    let mut frames = Vec::new();
    for i in 0..5 {
        let f = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timeout waiting for frame {i}"))
            .unwrap_or_else(|| panic!("stream ended early at frame {i}"));
        frames.push(f);
    }

    assert_eq!(frames[0].data, "one");
    assert_eq!(frames[0].id.as_deref(), Some("1"));
    assert_eq!(frames[1].data, "two");
    assert_eq!(frames[2].data, "three");
    assert_eq!(frames[2].id.as_deref(), Some("3"));
    assert_eq!(frames[3].data, "four");
    assert_eq!(frames[3].id.as_deref(), Some("4"));
    assert_eq!(frames[4].data, "five");

    // Allow a short tick for the server to register the 2nd GET's header.
    // (get_count is incremented at the top of the handler; the mutex insert
    // happens right after, so by the time we've seen frame 4, it's recorded.)
    let get_n = state.get_count.load(Ordering::SeqCst);
    assert_eq!(
        get_n, 2,
        "server should have observed exactly 2 GETs (initial + 1 reconnect), saw {get_n}"
    );

    let headers = state.last_event_ids.lock().await.clone();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0], "", "first GET should carry no Last-Event-ID");
    assert_eq!(
        headers[1], "3",
        "reconnect GET should carry Last-Event-ID: 3 (the last seen id before close)"
    );

    server.abort();
}

// ---------------------------------------------------------------------------
// Test 2: Persistent-down server hits max_retries and gives up cleanly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_persistent_down_exhausts_retries_and_ends() {
    use tokio::sync::mpsc;

    // Point the client at a port that nothing is listening on.
    let (tx, mut rx) = mpsc::channel::<archon_mcp::sse_transport::SseFrame>(64);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(200))
        .build()
        .expect("build reqwest client");

    let config = ReconnectConfig {
        default_retry_ms: 20,
        max_retries: 3, // small cap -> test stays fast
        jitter_ratio: 0.0,
    };

    // Bind + immediately drop a listener to grab an ephemeral port that
    // nothing will answer on for the test duration.
    let dead_addr = {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind for dead addr");
        listener.local_addr().expect("dead addr")
        // listener drops here
    };
    let url = format!("http://{dead_addr}/sse");

    let pump = tokio::spawn(pump_sse_stream_with_reconnect(
        client,
        url,
        std::collections::HashMap::new(),
        tx,
        config,
    ));

    // Receiver should get NO frames (server is dead) and the channel should
    // close once the pump exhausts retries.
    let first = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await;
    match first {
        Ok(None) => {
            // Channel closed after pump gave up — exactly what we want.
        }
        Ok(Some(f)) => panic!("unexpected frame on dead server: {f:?}"),
        Err(_) => panic!("pump should have given up well within 10s"),
    }

    // Pump task should have exited.
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("pump should have finished")
        .expect("pump should not panic");
}

// ---------------------------------------------------------------------------
// Test 3: retry: frame updates the backoff floor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_retry_field_updates_backoff() {
    // Test state: observe what the 2nd GET's timing implies about the retry
    // floor. This is a loose end-to-end check — we don't need exact timing,
    // just that a large server-requested retry doesn't collapse to the small
    // default.
    //
    // We do this at unit level in sse_reconnect::tests (compute_backoff and
    // frame state-update). This integration test just confirms the retry
    // frame is propagated into state by checking that a server-emitted
    // `retry: 30000` doesn't cause the client to reconnect within 1s of
    // close.
    use tokio::sync::mpsc;

    #[derive(Clone, Default)]
    struct State2 {
        get_count: Arc<AtomicU32>,
    }

    async fn handler2(State(state): State<State2>) -> Sse<BoxedEventStream> {
        let get_n = state.get_count.fetch_add(1, Ordering::SeqCst);
        let events: Vec<Event> = if get_n == 0 {
            vec![
                // retry: 30000 (30 seconds) — no reconnect should happen
                // inside the test's 1-second wait window.
                Event::default().retry(Duration::from_millis(30_000)).data("first"),
            ]
        } else {
            vec![Event::default().id("99").data("late-reconnect")]
        };
        let stream = stream::iter(events.into_iter().map(Ok::<_, Infallible>));
        let combined: BoxedEventStream = if get_n == 0 {
            Box::pin(stream) // will close after 1 event
        } else {
            Box::pin(stream.chain(futures_util::stream::pending()))
        };
        Sse::new(combined)
    }

    let state2 = State2::default();
    let app = Router::new()
        .route("/sse", get(handler2))
        .with_state(state2.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    let url = format!("http://{addr}/sse");

    let (tx, mut rx) = mpsc::channel::<archon_mcp::sse_transport::SseFrame>(64);
    let client = reqwest::Client::builder().build().unwrap();
    let config = ReconnectConfig {
        default_retry_ms: 50, // very short default
        max_retries: 10,
        jitter_ratio: 0.0,
    };
    tokio::spawn(pump_sse_stream_with_reconnect(
        client,
        url,
        std::collections::HashMap::new(),
        tx,
        config,
    ));

    // First frame should arrive quickly.
    let f1 = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .expect("first frame timeout")
        .expect("first frame");
    assert_eq!(f1.data, "first");
    assert_eq!(f1.retry, Some(30_000));

    // After the server closes, the pump should NOT have reconnected within
    // 1s — the server-requested retry of 30s dominates the 50ms default.
    let second = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
    assert!(
        second.is_err(),
        "should have timed out waiting — server-requested 30s retry should prevent fast reconnect"
    );

    let get_n = state2.get_count.load(Ordering::SeqCst);
    assert_eq!(
        get_n, 1,
        "server should have observed only the initial GET within the 1s window; saw {get_n}"
    );

    server.abort();
}
