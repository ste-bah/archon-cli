//! Channel observability instrumentation for AgentEvent channel.
//!
//! Provides backlog depth, throughput, and P95 send-to-render latency tracking,
//! plus the TASK-TUI-803 Prometheus `/metrics` exporter. The TASK-TUI-802
//! tracing plumbing (spans, redaction, `init_tracing`) lives in
//! `observability_tracing.rs` to keep this file under the 500-LoC ceiling
//! required by NFR-TUI-QUAL-001; it is re-exported below so the external API
//! (`archon_tui::observability::{init_tracing, span_agent_turn,
//! span_slash_dispatch, span_channel_send}`) is unchanged.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hdrhistogram::Histogram;
use parking_lot::Mutex;

// Re-export TASK-TUI-802 tracing surface so existing callers keep importing
// from `archon_tui::observability::…`. See `observability_tracing.rs`.
pub use crate::observability_tracing::{
    RedactionLayer, init_tracing, span_agent_turn, span_channel_send, span_slash_dispatch,
};

/// Channel instrumentation metrics.
///
/// # Atomic ordering
/// All counters use `Relaxed` ordering — these are approximate figures
/// for observability only, not correctness-critical data.
#[derive(Debug)]
pub struct ChannelMetrics {
    /// Current in-flight messages (sent - drained).
    pub backlog_depth: AtomicU64,
    /// Total messages sent since startup.
    pub total_sent: AtomicU64,
    /// Total messages drained since startup.
    pub total_drained: AtomicU64,
    /// Largest single drain batch observed.
    pub max_batch_size: AtomicU64,
    /// P95 send-to-render latency in milliseconds.
    pub p95_send_to_render_ms: Mutex<Histogram<u64>>,
    /// Timestamp of last WARN fire (unix ms). 0 == never fired.
    pub last_warn_unix_ms: AtomicU64,
}

impl ChannelMetrics {
    /// Construct a zeroed metrics instance.
    pub fn new() -> Self {
        Self {
            backlog_depth: AtomicU64::new(0),
            total_sent: AtomicU64::new(0),
            total_drained: AtomicU64::new(0),
            max_batch_size: AtomicU64::new(0),
            last_warn_unix_ms: AtomicU64::new(0),
            // histogram min=1ms, max=60_000ms (1 min), 3 significant figures
            p95_send_to_render_ms: Mutex::new(
                // Histogram range: 1ms floor (sub-ms rounded up), 60_000ms ceiling
                // (1 min — any higher is a stuck-thread bug, not a latency sample).
                Histogram::new_with_bounds(1, 60_000, 3)
                    .expect("histogram bounds 1..60_000ms, 3 sigfig — spec-constant"),
            ),
        }
    }

    /// Record a single send event.
    #[inline]
    pub fn record_sent(&self) {
        self.total_sent.fetch_add(1, Ordering::Relaxed);
        self.backlog_depth.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a drain event with the given batch size.
    #[inline]
    pub fn record_drained(&self, batch_size: u64) {
        // Cap batch_size at current backlog to prevent underflow
        let current_backlog = self.backlog_depth.load(Ordering::Relaxed);
        let actual_drained = batch_size.min(current_backlog);
        if actual_drained > 0 {
            self.total_drained.fetch_add(actual_drained, Ordering::Relaxed);
            self.backlog_depth.fetch_sub(actual_drained, Ordering::Relaxed);
        }
        self.max_batch_size.fetch_max(batch_size, Ordering::Relaxed);
    }

    /// Record a send-to-render latency sample in milliseconds.
    #[inline]
    pub fn record_latency_ms(&self, ms: u64) {
        let mut guard = self.p95_send_to_render_ms.lock();
        // Silently ignore recording errors — histogram has bounded range
        let _ = guard.record(ms);
    }

    /// Rate-limited backlog-depth warning gate.
    ///
    /// Returns `true` if a WARN should be emitted this call. Fires at most
    /// once per 1000 ms while `backlog_depth > threshold`. Uses
    /// `compare_exchange` on `last_warn_unix_ms` to avoid double-fire under
    /// concurrent races.
    #[inline]
    pub fn warn_if_backlog_over(&self, threshold: u64) -> bool {
        let backlog = self.backlog_depth.load(Ordering::Relaxed);
        if backlog <= threshold {
            return false;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last = self.last_warn_unix_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last) < 1000 {
            return false;
        }
        // compare_exchange to avoid double-fire under race
        if self.last_warn_unix_ms
            .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        tracing::warn!(
            backlog_depth = backlog,
            threshold = threshold,
            "AgentEvent channel backlog exceeded threshold"
        );
        true
    }

    /// Take an atomic snapshot of all counters.
    #[inline]
    pub fn snapshot(&self) -> ChannelMetricsSnapshot {
        ChannelMetricsSnapshot {
            backlog_depth: self.backlog_depth.load(Ordering::Relaxed),
            total_sent: self.total_sent.load(Ordering::Relaxed),
            total_drained: self.total_drained.load(Ordering::Relaxed),
            max_batch_size: self.max_batch_size.load(Ordering::Relaxed),
            p95_send_to_render_ms: {
                let guard = self.p95_send_to_render_ms.lock();
                guard.value_at_percentile(95.0) as u64
            },
        }
    }
}

impl Default for ChannelMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable snapshot of ChannelMetrics counters.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMetricsSnapshot {
    pub backlog_depth: u64,
    pub total_sent: u64,
    pub total_drained: u64,
    pub max_batch_size: u64,
    pub p95_send_to_render_ms: u64,
}

impl archon_core::ChannelMetricSink for ChannelMetrics {
    #[inline]
    fn record_sent(&self) {
        self.record_sent();
    }
    #[inline]
    fn record_drained(&self, batch_size: u64) {
        self.record_drained(batch_size);
    }
}

// ── TASK-TUI-803: Prometheus /metrics exporter ──────────────────────────────
//
// Renders a `ChannelMetricsSnapshot` into Prometheus text-format
// (`text/plain; version=0.0.4`) and serves it on a local HTTP endpoint when
// the operator passes `archon --metrics-port <PORT>`. Binds to `127.0.0.1`
// only — no 0.0.0.0, no TLS, no auth (per spec §Out of Scope).

/// Prometheus metric names. Kept together so `format_prometheus` and the
/// unit test stay in lock-step on renames.
const METRIC_BACKLOG_DEPTH: &str = "archon_tui_channel_backlog_depth";
const METRIC_TOTAL_SENT: &str = "archon_tui_channel_total_sent";
const METRIC_TOTAL_DRAINED: &str = "archon_tui_channel_total_drained";
const METRIC_MAX_BATCH_SIZE: &str = "archon_tui_channel_max_batch_size";
const METRIC_P95_LATENCY_MS: &str = "archon_tui_channel_p95_send_to_render_ms";

/// Format a `ChannelMetricsSnapshot` as Prometheus text exposition v0.0.4.
///
/// Each metric gets `# HELP`, `# TYPE`, and a single sample line.
/// Type selection (per https://prometheus.io/docs/concepts/metric_types/):
///   * `backlog_depth` — gauge (instantaneous, bidirectional).
///   * `total_sent` / `total_drained` — counter (monotonic cumulative; `_total`
///     suffix is the Prometheus convention for counters).
///   * `max_batch_size` — GAUGE, not counter. It is a `fetch_max` high-water
///     mark — scraper `rate()` on it is meaningless because the value does
///     not represent additive events. Counters require the value to be an
///     additive sum since process start. High-water marks are gauges.
///   * `p95_send_to_render_ms` — gauge (sampled quantile from histogram).
pub fn format_prometheus(snapshot: &ChannelMetricsSnapshot) -> String {
    let mut out = String::with_capacity(1024);
    // backlog_depth — instantaneous in-flight, so it is a gauge.
    out.push_str(&format!(
        "# HELP {METRIC_BACKLOG_DEPTH} Current AgentEvent channel backlog (sent - drained).\n"
    ));
    out.push_str(&format!("# TYPE {METRIC_BACKLOG_DEPTH} gauge\n"));
    out.push_str(&format!(
        "{METRIC_BACKLOG_DEPTH} {}\n",
        snapshot.backlog_depth
    ));
    // total_sent — monotonic since startup.
    out.push_str(&format!(
        "# HELP {METRIC_TOTAL_SENT} Total AgentEvents sent since process start.\n"
    ));
    out.push_str(&format!("# TYPE {METRIC_TOTAL_SENT} counter\n"));
    out.push_str(&format!("{METRIC_TOTAL_SENT} {}\n", snapshot.total_sent));
    // total_drained — monotonic since startup.
    out.push_str(&format!(
        "# HELP {METRIC_TOTAL_DRAINED} Total AgentEvents drained since process start.\n"
    ));
    out.push_str(&format!("# TYPE {METRIC_TOTAL_DRAINED} counter\n"));
    out.push_str(&format!(
        "{METRIC_TOTAL_DRAINED} {}\n",
        snapshot.total_drained
    ));
    // max_batch_size — high-water mark; GAUGE (not counter — not additive).
    out.push_str(&format!(
        "# HELP {METRIC_MAX_BATCH_SIZE} Largest drain batch observed since process start.\n"
    ));
    out.push_str(&format!("# TYPE {METRIC_MAX_BATCH_SIZE} gauge\n"));
    out.push_str(&format!(
        "{METRIC_MAX_BATCH_SIZE} {}\n",
        snapshot.max_batch_size
    ));
    // p95 — sampled gauge derived from the HDR histogram at snapshot time.
    out.push_str(&format!(
        "# HELP {METRIC_P95_LATENCY_MS} P95 send-to-render latency (milliseconds).\n"
    ));
    out.push_str(&format!("# TYPE {METRIC_P95_LATENCY_MS} gauge\n"));
    out.push_str(&format!(
        "{METRIC_P95_LATENCY_MS} {}\n",
        snapshot.p95_send_to_render_ms
    ));
    out
}

/// Minimal Prometheus text-format structural parser used by tests to validate
/// the exposition shape rather than accepting any substring hit. Returns, per
/// metric name, the declared type and the sample line suffix (value).
///
/// Rejects missing `# HELP`, missing `# TYPE`, orphan samples, and type
/// mismatches. Not a full Prometheus parser — only what `format_prometheus`
/// emits. Kept in-module so tests can treat it as an internal invariant.
#[cfg(test)]
fn parse_prometheus_exposition(
    body: &str,
) -> std::collections::HashMap<String, (String, String)> {
    use std::collections::HashMap;
    let mut help: HashMap<String, String> = HashMap::new();
    let mut types: HashMap<String, String> = HashMap::new();
    let mut samples: HashMap<String, String> = HashMap::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("# HELP ") {
            if let Some((name, txt)) = rest.split_once(' ') {
                help.insert(name.to_string(), txt.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("# TYPE ") {
            if let Some((name, ty)) = rest.split_once(' ') {
                types.insert(name.to_string(), ty.to_string());
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            // Sample line: `<name> <value>` (no labels in our emitter).
            if let Some((name, value)) = line.split_once(' ') {
                samples.insert(name.to_string(), value.to_string());
            }
        }
    }
    // Build (type, value) map; assert HELP existence as a side check.
    let mut out: HashMap<String, (String, String)> = HashMap::new();
    for (name, ty) in types {
        assert!(help.contains_key(&name), "metric {name} missing # HELP");
        let value = samples
            .remove(&name)
            .unwrap_or_else(|| panic!("metric {name} missing sample line"));
        out.insert(name, (ty, value));
    }
    assert!(
        samples.is_empty(),
        "orphan sample lines without # TYPE: {:?}",
        samples.keys().collect::<Vec<_>>(),
    );
    out
}

/// Build the `/metrics` router over a snapshot-per-scrape handler.
fn build_metrics_router(metrics: Arc<ChannelMetrics>) -> axum::Router {
    use axum::{Router, response::IntoResponse, routing::get};

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
        Ok(addr) => tracing::info!(%addr, "Prometheus /metrics exporter listening"),
        Err(e) => tracing::warn!(%e, "metrics exporter local_addr unavailable"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fresh() -> ChannelMetrics {
        ChannelMetrics::new()
    }

    #[test]
    fn new_returns_zeroed_backlog_depth() {
        let m = make_fresh();
        assert_eq!(m.backlog_depth.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn new_returns_zeroed_total_sent() {
        let m = make_fresh();
        assert_eq!(m.total_sent.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn new_returns_zeroed_total_drained() {
        let m = make_fresh();
        assert_eq!(m.total_drained.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn new_returns_zeroed_max_batch_size() {
        let m = make_fresh();
        assert_eq!(m.max_batch_size.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_sent_increments_backlog_and_total_sent() {
        let m = make_fresh();
        m.record_sent();
        assert_eq!(m.backlog_depth.load(Ordering::Relaxed), 1);
        assert_eq!(m.total_sent.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn snapshot_returns_current_values() {
        let m = make_fresh();
        m.record_sent();
        m.record_sent();
        m.record_drained(2);
        let snap = m.snapshot();
        assert_eq!(snap.backlog_depth, 0); // 2 sent - 2 drained
        assert_eq!(snap.total_sent, 2);
        assert_eq!(snap.total_drained, 2);
        assert_eq!(snap.max_batch_size, 2);
    }

    /// TASK-TUI-803 Gate 1: Prometheus exposition must parse as well-formed
    /// text-format (v0.0.4) — every metric has `# HELP`, `# TYPE`, a single
    /// sample line, and the declared type matches the spec's semantic role.
    /// Uses a structural parser rather than substring matching so malformed
    /// bodies (orphan samples, missing headers, duplicated types) FAIL here.
    #[test]
    fn format_prometheus_parses_and_types_match_semantics() {
        let m = make_fresh();
        m.record_sent();
        m.record_sent();
        m.record_sent();
        m.record_drained(2);
        m.record_latency_ms(7);

        let snap = m.snapshot();
        let body = format_prometheus(&snap);
        let parsed = parse_prometheus_exposition(&body);

        // Exactly 5 metrics expected — no more, no less.
        assert_eq!(
            parsed.len(),
            5,
            "expected 5 metrics, parsed {}: {:?}",
            parsed.len(),
            parsed.keys().collect::<Vec<_>>(),
        );

        // Each metric: declared type must match the Prometheus semantic role.
        let expected = [
            ("archon_tui_channel_backlog_depth", "gauge", snap.backlog_depth.to_string()),
            ("archon_tui_channel_total_sent", "counter", snap.total_sent.to_string()),
            ("archon_tui_channel_total_drained", "counter", snap.total_drained.to_string()),
            // max_batch_size is a high-water mark → GAUGE, not counter.
            ("archon_tui_channel_max_batch_size", "gauge", snap.max_batch_size.to_string()),
            (
                "archon_tui_channel_p95_send_to_render_ms",
                "gauge",
                snap.p95_send_to_render_ms.to_string(),
            ),
        ];
        for (name, want_ty, want_val) in expected {
            let (ty, val) = parsed
                .get(name)
                .unwrap_or_else(|| panic!("metric {name} missing from exposition; body=\n{body}"));
            assert_eq!(ty, want_ty, "{name} has type {ty}, want {want_ty}");
            assert_eq!(val, &want_val, "{name} sample value {val} != snapshot {want_val}");
        }
    }

    /// Regression guard: max_batch_size must NOT be declared `counter`. A
    /// scraper running `rate(archon_tui_channel_max_batch_size[5m])` on a
    /// counter-typed high-water gauge produces meaningless negative rates
    /// whenever the mark resets or holds flat. If anyone re-types this to
    /// `counter` the test fails loudly.
    #[test]
    fn max_batch_size_is_gauge_not_counter() {
        let body = format_prometheus(&make_fresh().snapshot());
        assert!(
            body.contains("# TYPE archon_tui_channel_max_batch_size gauge"),
            "max_batch_size must be gauge (high-water mark); body=\n{body}"
        );
        assert!(
            !body.contains("# TYPE archon_tui_channel_max_batch_size counter"),
            "max_batch_size must NOT be counter; body=\n{body}"
        );
    }
}
