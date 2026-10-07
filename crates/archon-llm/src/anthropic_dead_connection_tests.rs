//! Issue 364: a provider connection that dies while a response streams must
//! end the call with an operational stream error, promptly and without the
//! reader busy-polling the dead socket.
//!
//! Each case serves a real chunked SSE response on a local socket, then kills
//! it a different way. The reader runs on a current-thread runtime, as a
//! workflow script thread does, so a spin would also starve the heartbeat.
use super::AnthropicClient;
use crate::streaming::StreamEvent;
use futures_util::Stream;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Copy, Debug)]
enum Death {
    /// The peer closes the socket (FIN) mid-body.
    Close,
    /// The peer resets the socket (RST), as a proxy restarted after a wake does.
    Reset,
    /// The peer shuts down its write half and keeps the socket open.
    HalfClose,
    /// The peer keeps the socket open and sends nothing more.
    Silent,
}

async fn read_request(socket: &mut TcpStream) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    let header_end = loop {
        let read = socket.read(&mut buffer).await.expect("read request");
        assert!(read > 0, "client closed before its request headers");
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
    let length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while request.len() - header_end < length {
        let read = socket.read(&mut buffer).await.expect("read body");
        assert!(read > 0, "client closed before its request body");
        request.extend_from_slice(&buffer[..read]);
    }
}

fn chunk(payload: &str) -> String {
    format!("{:x}\r\n{payload}\r\n", payload.len())
}

/// Serve one streaming response that dies as `death` says. The returned
/// handle holds a socket the peer keeps open until the test drops it.
async fn serve_dying_stream(death: Death) -> (String, tokio::task::JoinHandle<Option<TcpStream>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        read_request(&mut socket).await;
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
        let start = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"model\":\"x\",\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n";
        let delta = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n";
        let body = format!("{head}{}{}", chunk(start), chunk(delta));
        socket.write_all(body.as_bytes()).await.expect("write");
        socket.flush().await.expect("flush");
        tokio::time::sleep(Duration::from_millis(50)).await;
        match death {
            Death::Close => None,
            Death::Reset => {
                socket.set_zero_linger().expect("zero linger");
                None
            }
            Death::HalfClose => {
                socket.shutdown().await.expect("shutdown write half");
                Some(socket)
            }
            Death::Silent => Some(socket),
        }
    });
    (url, server)
}

/// Counts how often the reader polls the response body.
struct CountingPolls<S> {
    inner: S,
    polls: Arc<AtomicU64>,
}

impl<S: Stream + Unpin> Stream for CountingPolls<S> {
    type Item = S::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::Relaxed);
        Pin::new(&mut this.inner).poll_next(cx)
    }
}

struct Outcome {
    events: Vec<StreamEvent>,
    elapsed: Duration,
    body_polls: u64,
    heartbeats: u64,
}

/// Open the stream exactly as `AnthropicClient::spawn_stream_reader` does and
/// read it to its end, under `limit`, beside a heartbeat on the same thread.
async fn read_dying_stream(death: Death, read_backstop_secs: u64, limit: Duration) -> Outcome {
    let (url, server) = serve_dying_stream(death).await;
    let client = AnthropicClient::with_read_backstop(
        crate::anthropic_tests::make_auth(),
        crate::anthropic_tests::make_identity(),
        Some(url.clone()),
        read_backstop_secs,
    );
    let response = client
        .http
        .post(&url)
        .body("{}")
        .send()
        .await
        .expect("send");
    let polls = Arc::new(AtomicU64::new(0));
    let mut rx = crate::anthropic_stream::spawn_anthropic_stream_reader(CountingPolls {
        inner: crate::transport_evidence::stream(response, Vec::new()),
        polls: Arc::clone(&polls),
    });
    let heartbeats = Arc::new(AtomicU64::new(0));
    let beat = {
        let heartbeats = Arc::clone(&heartbeats);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                heartbeats.fetch_add(1, Ordering::Relaxed);
            }
        })
    };
    let started = std::time::Instant::now();
    let mut events = Vec::new();
    let drained = tokio::time::timeout(limit, async {
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
    })
    .await;
    let elapsed = started.elapsed();
    beat.abort();
    let held = server.await.expect("server task");
    drop(held);
    assert!(
        drained.is_ok(),
        "{death:?}: the stream did not end within {limit:?}; events so far: {events:?}"
    );
    Outcome {
        events,
        elapsed,
        body_polls: polls.load(Ordering::Relaxed),
        heartbeats: heartbeats.load(Ordering::Relaxed),
    }
}

fn assert_operational_end(death: Death, outcome: &Outcome) {
    let last = outcome.events.last().expect("at least one event");
    assert!(
        matches!(last, StreamEvent::Error { error_type, .. }
            if matches!(error_type.as_str(), "network" | "protocol" | "transport_idle")),
        "{death:?}: a dead connection must end with an operational stream error, got {last:?}"
    );
    assert!(
        outcome
            .events
            .iter()
            .any(|event| matches!(event, StreamEvent::TextDelta { text, .. } if text == "partial")),
        "{death:?}: the delta before the death must still arrive"
    );
    // A reader that waits on the socket polls it a handful of times; one that
    // spins on a dead socket polls it without bound.
    assert!(
        outcome.body_polls < 50,
        "{death:?}: the reader polled the dead body {} times (busy loop)",
        outcome.body_polls
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_peer_close_mid_stream_ends_the_call_promptly() {
    let outcome = read_dying_stream(Death::Close, 30, Duration::from_secs(5)).await;
    assert_operational_end(Death::Close, &outcome);
    assert!(
        outcome.elapsed < Duration::from_secs(2),
        "{:?}",
        outcome.elapsed
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_peer_reset_mid_stream_ends_the_call_promptly() {
    let outcome = read_dying_stream(Death::Reset, 30, Duration::from_secs(5)).await;
    assert_operational_end(Death::Reset, &outcome);
    assert!(
        outcome.elapsed < Duration::from_secs(2),
        "{:?}",
        outcome.elapsed
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_peer_half_close_mid_stream_ends_the_call_promptly() {
    let outcome = read_dying_stream(Death::HalfClose, 30, Duration::from_secs(5)).await;
    assert_operational_end(Death::HalfClose, &outcome);
    assert!(
        outcome.elapsed < Duration::from_secs(2),
        "{:?}",
        outcome.elapsed
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_silent_peer_ends_at_the_read_backstop_without_spinning() {
    let outcome = read_dying_stream(Death::Silent, 1, Duration::from_secs(5)).await;
    assert_operational_end(Death::Silent, &outcome);
    assert!(
        matches!(outcome.events.last(), Some(StreamEvent::Error { error_type, .. }) if error_type == "transport_idle"),
        "a silent peer is a transport idle stall: {:?}",
        outcome.events.last()
    );
    // The thread stayed free while the reader waited: a free thread beats
    // ~100 times a second (~64 on a 15.6 ms Windows timer); a starved one not at all.
    assert!(
        outcome.heartbeats as f64 >= outcome.elapsed.as_secs_f64() * 20.0,
        "the reader starved its thread: {} beats in {:?}",
        outcome.heartbeats,
        outcome.elapsed
    );
}
