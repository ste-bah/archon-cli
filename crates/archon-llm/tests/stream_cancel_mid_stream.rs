//! TASK-P1-3 (#188) — streaming HTTP + mid-stream cancel smoke.
//!
//! Proves the reqwest/axum transport primitives used by archon-llm's
//! streaming providers (OpenAI-compat, Anthropic, etc.) correctly tear
//! down on client-side cancel — no leaked server connection, no panic.
//!
//! NOT a provider-level test. Exercises only the HTTP primitive layer:
//!   - axum SSE handler emits 3 tokens, then parks (pending forever)
//!   - reqwest client reads the 3 tokens, then drops the stream
//!   - RAII counter on the server side decrements when the stream future
//!     is dropped (client disconnect)
//!   - Assert active-connections returns to 0; peak was >=1
//!
//! Dep note (TASK-P1-3): `async-stream` is NOT a workspace dep and we
//! chose to avoid adding it. The SSE body stream is built from
//! `futures_util::stream::unfold` whose state owns a `ConnCounter`
//! guard — dropping the stream (client disconnect) drops the state,
//! which drops the guard, which decrements the connection counter.
//! This is the same drop-propagation behavior async-stream would give
//! us but without the extra dep.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use futures_util::{Stream, StreamExt, stream};
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct ConnState {
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

/// RAII counter that decrements `active` when dropped. Embedded in the
/// stream's unfold-state so it drops with the stream future on
/// client-side cancel.
struct ConnCounter {
    active: Arc<AtomicUsize>,
}

impl Drop for ConnCounter {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

/// unfold state: which step we're on, plus the RAII guard that lives as
/// long as the stream does.
struct Step {
    n: usize,
    _guard: ConnCounter,
}

/// SSE handler that emits 3 tokens, then stalls indefinitely (the
/// `stream::pending`-like tail) until the client drops the connection.
/// Increments `active` on entry, decrements on guard-drop.
async fn sse_handler(
    axum::extract::State(state): axum::extract::State<ConnState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let prev = state.active.fetch_add(1, Ordering::SeqCst);
    let new_count = prev + 1;
    state.peak.fetch_max(new_count, Ordering::SeqCst);

    let init = Step {
        n: 0,
        _guard: ConnCounter {
            active: state.active.clone(),
        },
    };

    // Produce tokens 0..3, then park forever via sleep. Dropping the
    // stream drops `Step` -> drops `ConnCounter` -> decrements `active`.
    let body = stream::unfold(init, |mut s| async move {
        if s.n < 3 {
            let ev = Event::default().data(format!("token{}", s.n));
            s.n += 1;
            // small gap between tokens so the client can reliably read
            // them one at a time under the timeout budget
            tokio::time::sleep(Duration::from_millis(10)).await;
            Some((Ok::<_, Infallible>(ev), s))
        } else {
            // Park: a long sleep yields control so the runtime can
            // observe the client-disconnect and drop this future.
            // KeepAlive drives wire-level pings while we're here.
            tokio::time::sleep(Duration::from_secs(3600)).await;
            None
        }
    });

    Sse::new(body).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_millis(100))
            .text("keepalive"),
    )
}

#[tokio::test]
async fn stream_cancel_mid_stream_drops_server_connection() {
    // Spin mock server on an ephemeral port.
    let state = ConnState::default();
    let app: Router = Router::new()
        .route("/stream", get(sse_handler))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Open stream.
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/stream"))
        .header("Accept", "text/event-stream")
        .send()
        .await
        .expect("connect");
    assert_eq!(resp.status(), 200);
    let mut bytes = resp.bytes_stream();

    // Read until we've seen exactly 3 "data: token" markers. Parses
    // loosely — SSE is newline-framed and tokens may arrive across
    // multiple chunks, so we count matches in concatenated text.
    let mut buf = String::new();
    let mut tokens_seen = 0usize;
    while tokens_seen < 3 {
        match tokio::time::timeout(Duration::from_secs(5), bytes.next()).await {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                tokens_seen = buf.matches("data: token").count();
            }
            Ok(Some(Err(e))) => panic!("stream error: {e}"),
            Ok(None) => panic!("stream ended before 3 tokens; buf={buf:?}"),
            Err(_) => panic!("timeout waiting for tokens; tokens_seen={tokens_seen}, buf={buf:?}"),
        }
    }
    assert!(
        tokens_seen >= 3,
        "must read at least 3 tokens before cancel; got {tokens_seen}"
    );

    // Server must show at least one active connection.
    let active_mid = state.active.load(Ordering::SeqCst);
    assert!(
        active_mid >= 1,
        "expected >=1 active server connection mid-stream; got {active_mid}"
    );

    // CANCEL: drop the byte stream AND the response.
    drop(bytes);
    // `resp` was consumed by `.bytes_stream()` so it's already gone.

    // Poll for the server-side decrement with a bounded budget — under
    // load it can take a tick or two for the runtime to observe the
    // disconnect and drop the handler future.
    let mut active_after = usize::MAX;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        active_after = state.active.load(Ordering::SeqCst);
        if active_after == 0 {
            break;
        }
    }
    assert_eq!(
        active_after, 0,
        "expected 0 active connections after client drop; got {active_after}"
    );

    // Peak >= 1 confirms we did reach a live state.
    let peak = state.peak.load(Ordering::SeqCst);
    assert!(peak >= 1, "peak active connections should be >= 1; got {peak}");

    server.abort();
}
