//! Tests for SSE reconnect pump shutdown via Notify.
//!
//! Uses the `axum::serve` + `tokio::net::TcpListener` pattern from
//! `sse_transport_reconnect.rs` (verified as the only available test dep).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream};
use tokio::net::TcpListener;
use tokio::sync::{Notify, mpsc};

use archon_mcp::sse_reconnect::{
    ReconnectConfig, pump_sse_stream_with_reconnect_with_shutdown, spawn_sse_pump,
};

// ---------------------------------------------------------------------------
// Mock server helpers
// ---------------------------------------------------------------------------

type BoxedEventStream = Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send + 'static>>;

async fn spawn_500_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/sse",
        get(|| async { axum::http::StatusCode::INTERNAL_SERVER_ERROR }),
    );
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, server)
}

async fn spawn_empty_stream_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    async fn handler() -> Sse<BoxedEventStream> {
        // One frame then immediate stream end — triggers post-stream backoff.
        let events = vec![Ok(Event::default().data("only"))];
        let stream: BoxedEventStream = Box::pin(stream::iter(events));
        Sse::new(stream)
    }
    let app = Router::new().route("/sse", get(handler));
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (addr, server)
}

// ---------------------------------------------------------------------------
// Test 1: Notify during backoff sleep exits promptly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_pump_exits_on_notify_during_backoff_sleep() {
    let (addr, _server) = spawn_500_server().await;
    let url = format!("http://{addr}/sse");

    let (tx, _rx) = mpsc::channel::<archon_mcp::sse_transport::SseFrame>(64);
    let client = reqwest::Client::builder()
        .build()
        .expect("build reqwest client");
    let config = ReconnectConfig {
        default_retry_ms: 10_000, // long so we're sure it's in the sleep
        max_retries: 5,
        jitter_ratio: 0.0,
    };
    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = Arc::clone(&shutdown);

    let handle = tokio::spawn(async move {
        pump_sse_stream_with_reconnect_with_shutdown(
            client,
            url,
            std::collections::HashMap::new(),
            tx,
            config,
            shutdown_clone,
        )
        .await;
    });

    // Let the pump hit the 500 -> enter backoff sleep.
    tokio::time::sleep(Duration::from_millis(200)).await;
    shutdown.notify_waiters();

    let result = tokio::time::timeout(Duration::from_millis(500), handle).await;
    assert!(
        result.is_ok(),
        "pump should exit within 500ms after notify during backoff sleep"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Notify during post-stream backoff exits promptly
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_pump_exits_on_notify_during_post_stream_sleep() {
    let (addr, _server) = spawn_empty_stream_server().await;
    let url = format!("http://{addr}/sse");

    let (tx, _rx) = mpsc::channel::<archon_mcp::sse_transport::SseFrame>(64);
    let client = reqwest::Client::builder()
        .build()
        .expect("build reqwest client");
    let config = ReconnectConfig {
        default_retry_ms: 10_000, // long backoff
        max_retries: 5,
        jitter_ratio: 0.0,
    };
    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = Arc::clone(&shutdown);

    let handle = tokio::spawn(async move {
        pump_sse_stream_with_reconnect_with_shutdown(
            client,
            url,
            std::collections::HashMap::new(),
            tx,
            config,
            shutdown_clone,
        )
        .await;
    });

    // Let the pump connect, drain the single frame, stream ends, enter backoff.
    tokio::time::sleep(Duration::from_millis(200)).await;
    shutdown.notify_waiters();

    let result = tokio::time::timeout(Duration::from_millis(500), handle).await;
    assert!(
        result.is_ok(),
        "pump should exit within 500ms after notify during post-stream backoff"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Drop on SseShutdown aborts the handle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_shutdown_drop_aborts_handle() {
    let get_count = Arc::new(AtomicU32::new(0));
    let get_count_clone = Arc::clone(&get_count);

    // This server counts GETs so we can verify the pump stops.
    let app = Router::new().route(
        "/sse",
        get(move || {
            let c = Arc::clone(&get_count_clone);
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }),
    );
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let _server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    let url = format!("http://{addr}/sse");

    let (tx, _rx) = mpsc::channel::<archon_mcp::sse_transport::SseFrame>(64);
    let client = reqwest::Client::builder()
        .build()
        .expect("build reqwest client");
    let config = ReconnectConfig {
        default_retry_ms: 50,
        max_retries: 10,
        jitter_ratio: 0.0,
    };

    let guard = spawn_sse_pump(client, url, std::collections::HashMap::new(), tx, config);

    // Let the pump make a few GET attempts.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = get_count.load(Ordering::SeqCst);
    assert!(before > 0, "pump should have made at least one GET");

    drop(guard);

    // After drop, wait and verify the counter stops incrementing.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after_drop = get_count.load(Ordering::SeqCst);

    tokio::time::sleep(Duration::from_millis(500)).await;
    let final_count = get_count.load(Ordering::SeqCst);

    // After drop, the counter should not increase significantly.
    assert!(
        final_count - after_drop <= 1,
        "pump should stop issuing GETs after guard drop: before={before}, after_drop={after_drop}, final={final_count}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Graceful shutdown returns within timeout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_shutdown_graceful_returns_within_timeout() {
    let (addr, _server) = spawn_500_server().await;
    let url = format!("http://{addr}/sse");

    let (tx, _rx) = mpsc::channel::<archon_mcp::sse_transport::SseFrame>(64);
    let client = reqwest::Client::builder()
        .build()
        .expect("build reqwest client");
    let config = ReconnectConfig {
        default_retry_ms: 50,
        max_retries: 5,
        jitter_ratio: 0.0,
    };

    let guard = spawn_sse_pump(client, url, std::collections::HashMap::new(), tx, config);

    // Let the pump start.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(Duration::from_millis(2500), guard.shutdown()).await;
    assert!(
        result.is_ok(),
        "graceful shutdown must complete within 2.5s"
    );
}
