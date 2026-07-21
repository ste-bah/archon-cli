//! SSE reconnect + Last-Event-ID replay (#202 MCP-SSE-HARDEN-RETRY).
//!
//! Layered on top of the one-shot frame pump in [`crate::sse_transport`]:
//! this module owns the full lifecycle of a reconnecting SSE consumer.
//!
//! Responsibilities:
//!   * Open a fresh SSE GET on startup and after any stream termination.
//!   * Track the most recent `id:` from received frames; include it as
//!     `Last-Event-ID` on reconnect GETs so the server can replay missed
//!     events (servers that don't support replay ignore the header).
//!   * Parse `retry:` frames to learn the server-suggested reconnect
//!     interval; clamp with an internal max of 60s.
//!   * Exponential backoff with ±jitter on consecutive failures; reset
//!     the attempt counter after any successful connection.
//!   * Cap retries at [`ReconnectConfig::max_retries`] — persistent-down
//!     servers surface as a closed channel to the caller.
//!
//! # Out of scope
//!
//! * Server-side session coordination (the `Mcp-Session-Id` header) —
//!   that's #203 and lives in `sse_mcp_transport`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use http::{HeaderName, HeaderValue};
use rand::Rng;
use tokio::sync::{Notify, mpsc};

use crate::sse_transport::{SseFrame, SseFrameBuilder};

/// Tunables for the reconnecting SSE pump.
#[derive(Debug, Clone, Copy)]
pub struct ReconnectConfig {
    /// Initial retry interval in milliseconds (per SSE spec default: 3000).
    /// Overridden at runtime whenever the server emits a `retry:` frame.
    pub default_retry_ms: u64,
    /// Maximum number of consecutive reconnect attempts before giving up.
    /// After a successful connection this counter resets.
    pub max_retries: u32,
    /// Jitter ratio applied to each backoff interval. `0.25` means
    /// `backoff * [0.875, 1.125]`. `0.0` disables jitter (useful for tests).
    pub jitter_ratio: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            default_retry_ms: 3_000,
            max_retries: 10,
            jitter_ratio: 0.25,
        }
    }
}

/// Runtime state mutated as frames are processed.
#[derive(Debug, Default, Clone)]
pub struct ReconnectState {
    /// Most recent `id:` value seen on any data frame.
    pub last_event_id: Option<String>,
    /// Current retry interval in milliseconds (updated by `retry:` frames).
    pub retry_ms: u64,
}

/// Upper bound for the exponential backoff delay — prevents runaway waits
/// even if the server pushes a very large `retry:` value.
const BACKOFF_CAP_MS: u64 = 60_000;

/// Maximum shift used in exponential backoff (`1 << SHIFT_CAP` = 64x base).
const SHIFT_CAP: u32 = 6;

/// Maximum bytes retained while waiting for an SSE line terminator.
const MAX_SSE_LINE_BUFFER_BYTES: usize = 1_024 * 1_024;

fn append_sse_line_bytes(buffer: &mut Vec<u8>, pending_cr: &mut bool, segment: &[u8]) -> bool {
    let terminated = segment.last() == Some(&b'\n');
    let mut content = segment.strip_suffix(b"\n").unwrap_or(segment);
    let previous_cr_is_delimiter = *pending_cr && segment.first() == Some(&b'\n');
    let append_previous_cr = *pending_cr && !previous_cr_is_delimiter;
    let defer_current_cr = !terminated && content.last() == Some(&b'\r');
    if (defer_current_cr || terminated) && content.last() == Some(&b'\r') {
        content = &content[..content.len() - 1];
    }

    let additional = content.len() + usize::from(append_previous_cr);
    let Some(next_len) = buffer.len().checked_add(additional) else {
        return false;
    };
    if next_len > MAX_SSE_LINE_BUFFER_BYTES {
        return false;
    }

    if append_previous_cr {
        buffer.push(b'\r');
    }
    buffer.extend_from_slice(content);
    *pending_cr = defer_current_cr;
    true
}

/// Compute the next reconnect delay from the current `retry_ms` base,
/// the consecutive-failure `attempt` count, and a jitter ratio.
///
/// Formula: `base = min(retry_ms * 2^(attempt-1), 60_000)`; apply ±jitter.
pub fn compute_backoff(retry_ms: u64, attempt: u32, jitter_ratio: f64) -> Duration {
    if attempt == 0 {
        return Duration::from_millis(retry_ms.min(BACKOFF_CAP_MS));
    }
    let shift = attempt.saturating_sub(1).min(SHIFT_CAP);
    let base_ms = retry_ms.saturating_mul(1u64 << shift).min(BACKOFF_CAP_MS);

    if jitter_ratio <= 0.0 || base_ms == 0 {
        return Duration::from_millis(base_ms);
    }
    let jitter_range = ((base_ms as f64) * jitter_ratio).round() as u64;
    if jitter_range == 0 {
        return Duration::from_millis(base_ms);
    }
    let low = base_ms.saturating_sub(jitter_range / 2);
    let high = base_ms.saturating_add(jitter_range / 2);
    let span = high.saturating_sub(low);
    if span == 0 {
        return Duration::from_millis(base_ms);
    }
    let mut rng = rand::rng();
    let jitter = rng.random_range(0..=span);
    Duration::from_millis(low.saturating_add(jitter))
}

/// Result of a single streaming session.
enum PumpOutcome {
    /// Caller dropped the receiver — stop the pump entirely.
    ReceiverDropped,
    /// Stream ended (natural EOF or error) — try to reconnect.
    StreamEnded,
}

/// Reconnecting SSE pump. Runs until the caller drops the `tx` receiver
/// or `config.max_retries` is exhausted on consecutive failures.
///
/// This is a backward-compat wrapper. Production code should use
/// [`spawn_sse_pump`] which wires a real `Notify` for prompt cancel.
pub async fn pump_sse_stream_with_reconnect(
    client: reqwest::Client,
    url: String,
    base_headers: HashMap<HeaderName, HeaderValue>,
    tx: mpsc::Sender<SseFrame>,
    config: ReconnectConfig,
) {
    let shutdown = Arc::new(Notify::new());
    pump_sse_stream_with_reconnect_with_shutdown(client, url, base_headers, tx, config, shutdown)
        .await
}

/// Cancel-aware variant. The 5-arg [`pump_sse_stream_with_reconnect`]
/// delegates to this with a never-fired Notify for backward compat
/// with existing test callers.
pub async fn pump_sse_stream_with_reconnect_with_shutdown(
    client: reqwest::Client,
    url: String,
    base_headers: HashMap<HeaderName, HeaderValue>,
    tx: mpsc::Sender<SseFrame>,
    config: ReconnectConfig,
    shutdown: Arc<Notify>,
) {
    let mut state = ReconnectState {
        retry_ms: config.default_retry_ms,
        last_event_id: None,
    };
    let mut attempt: u32 = 0;

    loop {
        // Build the GET request with Accept + base headers + optional Last-Event-ID.
        let mut req = client
            .get(&url)
            .header(http::header::ACCEPT, "text/event-stream");
        for (name, value) in &base_headers {
            req = req.header(name.clone(), value.clone());
        }
        if let Some(id) = &state.last_event_id
            && let Ok(v) = HeaderValue::from_str(id)
        {
            req = req.header("Last-Event-ID", v);
        }

        // Try to open the stream.
        let resp = match req.send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::warn!(status = %r.status(), url = %url, "SSE GET non-2xx");
                attempt += 1;
                if attempt > config.max_retries {
                    tracing::error!(
                        max = config.max_retries,
                        "SSE reconnect exhausted retries; giving up"
                    );
                    return;
                }
                let delay = compute_backoff(state.retry_ms, attempt, config.jitter_ratio);
                tokio::select! {
                    biased;
                    _ = shutdown.notified() => {
                        tracing::debug!("sse reconnect: shutdown notified, exiting");
                        return;
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
                continue;
            }
            Err(e) => {
                tracing::warn!(error = %e, url = %url, "SSE GET send failed");
                attempt += 1;
                if attempt > config.max_retries {
                    tracing::error!(
                        max = config.max_retries,
                        "SSE reconnect exhausted retries; giving up"
                    );
                    return;
                }
                let delay = compute_backoff(state.retry_ms, attempt, config.jitter_ratio);
                tokio::select! {
                    biased;
                    _ = shutdown.notified() => {
                        tracing::debug!("sse reconnect: shutdown notified, exiting");
                        return;
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
                continue;
            }
        };

        // Successful connect — reset the consecutive-failure counter.
        attempt = 0;

        match pump_one_stream_with_state(resp, &tx, &mut state).await {
            PumpOutcome::ReceiverDropped => return,
            PumpOutcome::StreamEnded => {
                attempt += 1;
                if attempt > config.max_retries {
                    tracing::error!(
                        max = config.max_retries,
                        "SSE reconnect exhausted retries after stream end; giving up"
                    );
                    return;
                }
                let delay = compute_backoff(state.retry_ms, attempt, config.jitter_ratio);
                tracing::info!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    last_event_id = ?state.last_event_id,
                    "SSE reconnecting after stream end"
                );
                tokio::select! {
                    biased;
                    _ = shutdown.notified() => {
                        tracing::debug!("sse reconnect: shutdown notified, exiting");
                        return;
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

/// Pump ONE reqwest response stream, parsing SSE frames and forwarding them
/// to `tx`. Updates `state.last_event_id` and `state.retry_ms` as frames
/// arrive so the caller's subsequent reconnect carries the right replay
/// cursor and honors the server-requested backoff.
async fn pump_one_stream_with_state(
    resp: reqwest::Response,
    tx: &mpsc::Sender<SseFrame>,
    state: &mut ReconnectState,
) -> PumpOutcome {
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut pending_cr = false;
    let mut current = SseFrameBuilder::default();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "SSE stream read error; will reconnect");
                return PumpOutcome::StreamEnded;
            }
        };
        for segment in chunk.split_inclusive(|byte| *byte == b'\n') {
            if !append_sse_line_bytes(&mut buf, &mut pending_cr, segment) {
                tracing::warn!(
                    limit_bytes = MAX_SSE_LINE_BUFFER_BYTES,
                    "SSE line buffer exceeded limit; will reconnect"
                );
                return PumpOutcome::StreamEnded;
            }
            if segment.last() != Some(&b'\n') {
                continue;
            }

            let line_len = buf.len();

            match std::str::from_utf8(&buf[..line_len]) {
                Ok("") => {
                    if let Some(frame) = current.take_frame() {
                        if let Some(id) = &frame.id {
                            state.last_event_id = Some(id.clone());
                        }
                        if let Some(r) = frame.retry {
                            state.retry_ms = r;
                        }
                        if tx.send(frame).await.is_err() {
                            return PumpOutcome::ReceiverDropped;
                        }
                    }
                }
                Ok(line) if line.starts_with(':') => {}
                Ok(line) => current.ingest_line(line),
                Err(e) => tracing::warn!(error = %e, "non-UTF8 SSE line; skipping"),
            }
            buf.clear();
        }
    }

    // Drain dangling frame without trailing blank line.
    if let Some(frame) = current.take_frame() {
        if let Some(id) = &frame.id {
            state.last_event_id = Some(id.clone());
        }
        if let Some(r) = frame.retry {
            state.retry_ms = r;
        }
        let _ = tx.send(frame).await;
    }

    PumpOutcome::StreamEnded
}

// Re-exported from sse_shutdown module (split for file-size budget).
pub use crate::sse_shutdown::{SseGuardedStream, SseShutdown, spawn_sse_pump};

#[cfg(test)]
#[path = "sse_reconnect_tests.rs"]
mod tests;
