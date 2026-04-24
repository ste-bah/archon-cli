//! `/metrics` HTTP exposition: [`serve_metrics`] and [`serve_metrics_on`].
//!
//! Split out of the original single-file `metrics.rs` in OBS-SPLIT-METRICS.
//! Public API is byte-for-byte preserved via re-exports in the parent
//! `metrics/mod.rs` — downstream callers keep writing
//! `archon_observability::metrics::serve_metrics`, etc.
//!
//! The HTTP handler snapshots a shared [`ChannelMetrics`] per scrape and
//! renders it through [`format_prometheus`] — both imported from the
//! sibling `channel` module.

use std::sync::Arc;

use super::channel::{format_prometheus, ChannelMetrics};

/// Build the `/metrics` router over a snapshot-per-scrape handler.
fn build_metrics_router(metrics: Arc<ChannelMetrics>) -> axum::Router {
    use axum::{response::IntoResponse, routing::get, Router};

    let metrics_for_handler = Arc::clone(&metrics);
    let handler = move || {
        let metrics = Arc::clone(&metrics_for_handler);
        async move {
            let snapshot = metrics.snapshot();
            let body = format_prometheus(&snapshot);
            (
                // Idiomatic mixed-case `Content-Type` — HTTP header names are
                // case-insensitive per RFC 7230 §3.2, but mixed-case matches
                // the rest of the codebase and the form Prometheus itself
                // emits in request headers.
                [("Content-Type", "text/plain; version=0.0.4; charset=utf-8")],
                body,
            )
                .into_response()
        }
    };
    Router::new().route("/metrics", get(handler))
}

/// Serve `/metrics` over an **already bound** listener.
///
/// This is the preferred entrypoint when bind errors must be observable by
/// the caller — the CLI binds synchronously to turn "permission denied" or
/// "address in use" into `Err` before a task is spawned. Runtime failures
/// during serve (peer reset, listener EOF) still bubble out via the
/// returned future.
pub async fn serve_metrics_on(
    listener: tokio::net::TcpListener,
    metrics: Arc<ChannelMetrics>,
) -> anyhow::Result<()> {
    let app = build_metrics_router(metrics);
    match listener.local_addr() {
        Ok(addr) => ::tracing::info!(%addr, "Prometheus /metrics exporter listening"),
        Err(e) => ::tracing::warn!(%e, "metrics exporter local_addr unavailable"),
    }
    axum::serve(listener, app).await?;
    Ok(())
}

/// Convenience wrapper: bind + serve in one future. Retained for callers
/// that do not need synchronous bind-error observation (tests, adhoc uses).
/// Production CLI path uses `serve_metrics_on` with pre-bound listener.
pub async fn serve_metrics(port: u16, metrics: Arc<ChannelMetrics>) -> anyhow::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        anyhow::anyhow!("failed to bind metrics exporter on {addr}: {e}")
    })?;
    serve_metrics_on(listener, metrics).await
}
