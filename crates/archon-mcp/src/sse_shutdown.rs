//! Shutdown guard and RAII stream wrapper for the SSE reconnect pump.
//!
//! Split from `sse_reconnect.rs` to stay under the 500-line file-size budget.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::Stream;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::sse_reconnect::{ReconnectConfig, pump_sse_stream_with_reconnect_with_shutdown};
use crate::sse_transport::SseFrame;

/// Owns the lifecycle of a spawned SSE reconnect pump task.
///
/// The owner MUST either call [`shutdown`](Self::shutdown) before the runtime
/// tears down for graceful termination, OR let `Drop` fire — Drop signals
/// Notify and aborts the handle as a fallback.
pub struct SseShutdown {
    notify: Arc<Notify>,
    handle: Option<JoinHandle<()>>,
}

impl SseShutdown {
    /// Graceful shutdown: wake the pump via Notify and `.await` the join
    /// handle with a 2-second timeout.
    pub async fn shutdown(mut self) {
        self.notify.notify_waiters();
        if let Some(h) = self.handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
        }
    }

    /// Return a clone of the inner `Notify` so callers can `select!` against
    /// the same notify the pump uses (e.g. for handshake-timeout wrapping).
    pub fn shutdown_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.notify)
    }
}

impl Drop for SseShutdown {
    fn drop(&mut self) {
        self.notify.notify_waiters();
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// Spawn the SSE reconnect pump and return its lifecycle guard.
pub fn spawn_sse_pump(
    client: reqwest::Client,
    url: String,
    base_headers: std::collections::HashMap<http::HeaderName, http::HeaderValue>,
    tx: mpsc::Sender<SseFrame>,
    config: ReconnectConfig,
) -> SseShutdown {
    let notify = Arc::new(Notify::new());
    let handle = tokio::spawn(pump_sse_stream_with_reconnect_with_shutdown(
        client,
        url,
        base_headers,
        tx,
        config,
        Arc::clone(&notify),
    ));
    SseShutdown {
        notify,
        handle: Some(handle),
    }
}

/// RAII wrapper that ties an SSE stream's lifetime to an [`SseShutdown`]
/// guard. When the wrapped stream is dropped (e.g. when the rmcp
/// `RunningService` shuts down), the guard's `Drop` fires and the
/// SSE pump exits cleanly.
///
/// # Field declaration order matters
///
/// Rust drops fields in declaration order (RFC 1857). `inner` drops
/// FIRST, closing the `SseFrame` channel; the pump observes
/// `PumpOutcome::ReceiverDropped` on its next `tx.send().await` and
/// returns cleanly. `_guard` then drops, firing
/// `notify_waiters() + handle.abort()` on a task that is already
/// exiting; both operations are idempotent no-ops in that state.
/// Declaration order is therefore correct as written — do not swap.
pub struct SseGuardedStream<S> {
    inner: S,
    _guard: SseShutdown,
}

impl<S> SseGuardedStream<S> {
    pub fn new(inner: S, guard: SseShutdown) -> Self {
        Self {
            inner,
            _guard: guard,
        }
    }
}

impl<S: Stream> Stream for SseGuardedStream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: `inner` is structurally pinned — we never move it
        // out of `Self`, never expose `&mut inner` outside this
        // method, and never observe `inner` in a non-pinned context.
        // `_guard` is `Unpin` (Arc + Option<JoinHandle>) and does
        // not require projection. This is the standard manual Pin
        // projection pattern for a single pinned field.
        let inner = unsafe { self.map_unchecked_mut(|s| &mut s.inner) };
        inner.poll_next(cx)
    }
}
