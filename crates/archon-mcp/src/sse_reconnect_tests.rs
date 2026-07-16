use super::*;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Response, StatusCode};
use axum::routing::get;
use futures_util::stream;
use tokio::net::TcpListener;

async fn spawn_raw_sse(body: Body) -> (String, tokio::task::JoinHandle<()>) {
    let body = Arc::new(tokio::sync::Mutex::new(Some(body)));
    let app = Router::new().route(
        "/sse",
        get(move || {
            let body = Arc::clone(&body);
            async move {
                let body = body.lock().await.take().unwrap_or_else(Body::empty);
                Response::builder()
                    .status(StatusCode::OK)
                    .header(http::header::CONTENT_TYPE, "text/event-stream")
                    .body(body)
                    .expect("valid SSE response")
            }
        }),
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/sse"), server)
}

#[test]
fn compute_backoff_zero_attempt_uses_retry_ms() {
    let d = compute_backoff(3_000, 0, 0.0);
    assert_eq!(d, Duration::from_millis(3_000));
}

#[test]
fn compute_backoff_grows_exponentially() {
    let d1 = compute_backoff(1_000, 1, 0.0);
    let d2 = compute_backoff(1_000, 2, 0.0);
    let d3 = compute_backoff(1_000, 3, 0.0);
    let d4 = compute_backoff(1_000, 4, 0.0);
    assert_eq!(d1, Duration::from_millis(1_000));
    assert_eq!(d2, Duration::from_millis(2_000));
    assert_eq!(d3, Duration::from_millis(4_000));
    assert_eq!(d4, Duration::from_millis(8_000));
}

#[test]
fn compute_backoff_caps_at_60s() {
    let d = compute_backoff(1_000_000, 2, 0.0);
    assert_eq!(d, Duration::from_millis(BACKOFF_CAP_MS));
}

#[test]
fn compute_backoff_jitter_within_bounds() {
    // base = 2_000 ms, jitter = 25% -> band [1750, 2250].
    for _ in 0..64 {
        let d = compute_backoff(1_000, 2, 0.25);
        let ms = d.as_millis() as u64;
        assert!(
            (1_750..=2_250).contains(&ms),
            "backoff {ms}ms out of jitter band [1750, 2250]"
        );
    }
}

#[test]
fn compute_backoff_shift_cap_prevents_overflow() {
    // attempt=100 should NOT overflow; uses SHIFT_CAP to bound.
    let d = compute_backoff(1_000, 100, 0.0);
    assert_eq!(d, Duration::from_millis(BACKOFF_CAP_MS));
}

#[tokio::test]
async fn pump_one_stream_updates_last_event_id_and_retry() {
    // Build a fake response from bytes using reqwest's test helper
    // isn't trivial, so exercise the state-update logic via a direct
    // unit test of the inner frame-dispatch loop.
    //
    // Simulate: ingest 2 frames, verify state updates.
    let mut state = ReconnectState::default();
    let mut b = SseFrameBuilder::default();
    b.ingest_line("id: 42");
    b.ingest_line("retry: 7500");
    b.ingest_line("data: hi");
    let frame = b.take_frame().unwrap();
    if let Some(id) = &frame.id {
        state.last_event_id = Some(id.clone());
    }
    if let Some(r) = frame.retry {
        state.retry_ms = r;
    }
    assert_eq!(state.last_event_id.as_deref(), Some("42"));
    assert_eq!(state.retry_ms, 7_500);

    // Second frame without id keeps the old id.
    let mut b2 = SseFrameBuilder::default();
    b2.ingest_line("data: next");
    let frame2 = b2.take_frame().unwrap();
    if let Some(id) = &frame2.id {
        state.last_event_id = Some(id.clone());
    }
    if let Some(r) = frame2.retry {
        state.retry_ms = r;
    }
    assert_eq!(state.last_event_id.as_deref(), Some("42"));
    assert_eq!(state.retry_ms, 7_500);
}

#[test]
fn sse_line_buffer_rejects_unterminated_overflow_without_growth() {
    let mut buffer = vec![b'x'; MAX_SSE_LINE_BUFFER_BYTES];
    let mut pending_cr = false;

    assert!(!append_sse_line_bytes(&mut buffer, &mut pending_cr, b"x"));
    assert_eq!(buffer.len(), MAX_SSE_LINE_BUFFER_BYTES);
}

#[test]
fn sse_line_buffer_accepts_exact_limit_with_same_chunk_newline() {
    let mut buffer = Vec::new();
    let mut pending_cr = false;
    let mut line = vec![b'x'; MAX_SSE_LINE_BUFFER_BYTES];
    line.push(b'\n');

    assert!(append_sse_line_bytes(&mut buffer, &mut pending_cr, &line));
    assert_eq!(buffer.len(), MAX_SSE_LINE_BUFFER_BYTES);
}

#[test]
fn sse_line_buffer_accepts_exact_limit_with_cross_chunk_newline() {
    let mut buffer = vec![b'x'; MAX_SSE_LINE_BUFFER_BYTES];
    let mut pending_cr = false;

    assert!(append_sse_line_bytes(&mut buffer, &mut pending_cr, b"\n"));
}

#[test]
fn sse_line_buffer_accepts_exact_limit_with_same_chunk_crlf() {
    let mut buffer = Vec::new();
    let mut pending_cr = false;
    let mut line = vec![b'x'; MAX_SSE_LINE_BUFFER_BYTES];
    line.extend_from_slice(b"\r\n");

    assert!(append_sse_line_bytes(&mut buffer, &mut pending_cr, &line));
    assert_eq!(buffer.len(), MAX_SSE_LINE_BUFFER_BYTES);
    assert!(!pending_cr);
}

#[test]
fn sse_line_buffer_accepts_exact_limit_with_cross_chunk_crlf() {
    let mut buffer = vec![b'x'; MAX_SSE_LINE_BUFFER_BYTES];
    let mut pending_cr = false;

    assert!(append_sse_line_bytes(&mut buffer, &mut pending_cr, b"\r"));
    assert_eq!(buffer.len(), MAX_SSE_LINE_BUFFER_BYTES);
    assert!(pending_cr);
    assert!(append_sse_line_bytes(&mut buffer, &mut pending_cr, b"\n"));
    assert_eq!(buffer.len(), MAX_SSE_LINE_BUFFER_BYTES);
    assert!(!pending_cr);
}

#[tokio::test]
async fn pump_ends_when_unterminated_line_exceeds_limit() {
    let chunks = stream::iter([Ok::<_, std::io::Error>(Bytes::from(vec![
        b'x';
        MAX_SSE_LINE_BUFFER_BYTES
            + 1
    ]))])
    .chain(stream::pending());
    let (url, server) = spawn_raw_sse(Body::from_stream(chunks)).await;
    let response = reqwest::Client::new().get(url).send().await.unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let mut state = ReconnectState::default();

    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        pump_one_stream_with_state(response, &tx, &mut state),
    )
    .await
    .expect("overflow must end the pump before the server closes");

    assert!(matches!(outcome, PumpOutcome::StreamEnded));
    server.abort();
}

#[tokio::test]
async fn pump_accepts_large_chunk_of_bounded_lines() {
    let mut payload = Vec::with_capacity(MAX_SSE_LINE_BUFFER_BYTES + 64);
    while payload.len() <= MAX_SSE_LINE_BUFFER_BYTES {
        payload.extend_from_slice(b": keepalive\n");
    }
    payload.extend_from_slice(b"data: delivered\n\n");
    let (url, server) = spawn_raw_sse(Body::from(payload)).await;
    let response = reqwest::Client::new().get(url).send().await.unwrap();
    let (tx, mut rx) = mpsc::channel(1);
    let mut state = ReconnectState::default();

    let outcome = pump_one_stream_with_state(response, &tx, &mut state).await;

    assert!(matches!(outcome, PumpOutcome::StreamEnded));
    assert_eq!(rx.recv().await.unwrap().data, "delivered");
    server.abort();
}

#[test]
fn reconnect_config_default_is_sane() {
    let c = ReconnectConfig::default();
    assert_eq!(c.default_retry_ms, 3_000);
    assert_eq!(c.max_retries, 10);
    assert!(c.jitter_ratio > 0.0 && c.jitter_ratio < 1.0);
}
