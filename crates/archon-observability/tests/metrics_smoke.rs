//! OBS-901 LIFT smoke test.
//!
//! Written BEFORE the LIFT lands (dev-flow Gate 1). Pins the post-LIFT
//! public surface at TWO independent boundaries so a future silent
//! refactor of the metrics module cannot break either without a compile
//! error at the external-crate boundary:
//!
//!   1. **archon-observability direct surface** — `ChannelMetrics`,
//!      `ChannelMetricsSnapshot`, `ChannelMetricSink`, `format_prometheus`
//!      at `archon_observability::metrics::…`.
//!
//!   2. **archon-observability crate-root re-exports** — same symbols at
//!      `archon_observability::…`. Downstream callers use the short
//!      paths; the re-exports must stay stable so the OBS-906 pattern
//!      (where an internal reshuffle doesn't leak out) keeps working.
//!
//! The archon-core back-compat shim (`pub use
//! archon_observability::ChannelMetricSink;` in `archon-core/src/metrics.rs`)
//! cannot be tested from this file because adding archon-core as a dep of
//! archon-observability would create a cycle (archon-core already depends
//! on archon-observability for the trait source). That back-compat contract
//! is pinned by `crates/archon-tui/tests/channel_metrics_wiring_test.rs`,
//! which legitimately depends on both crates and imports
//! `archon_core::ChannelMetricSink` in its first `use` line.
//!
//! Behavioural contracts (counter arithmetic, Prometheus format, P95
//! histogram bounds) are covered by the unit tests that move alongside
//! the impl into `src/metrics.rs`. This file is SHAPE coverage only.

/// `ChannelMetrics` + `ChannelMetricsSnapshot` must be reachable at the
/// submodule path and must round-trip record → snapshot.
#[test]
fn metrics_submodule_path_reachable() {
    let m = archon_observability::metrics::ChannelMetrics::new();
    m.record_sent();
    m.record_sent();
    m.record_drained(2);
    let snap: archon_observability::metrics::ChannelMetricsSnapshot = m.snapshot();
    assert_eq!(snap.total_sent, 2);
    assert_eq!(snap.total_drained, 2);
    assert_eq!(snap.backlog_depth, 0);
}

/// Crate-root re-export for `ChannelMetrics` keeps the short path stable
/// so downstream crates don't have to chase submodule moves.
#[test]
fn crate_root_channel_metrics_reexport_stable() {
    let m = archon_observability::ChannelMetrics::default();
    let snap: archon_observability::ChannelMetricsSnapshot = m.snapshot();
    assert_eq!(snap.total_sent, 0);
    assert_eq!(snap.backlog_depth, 0);
}

/// `ChannelMetricSink` trait must be reachable at the new crate-root path.
/// This is the trait implementors use to accept a generic metrics sink.
#[test]
fn crate_root_channel_metric_sink_trait_reachable() {
    use archon_observability::ChannelMetricSink;
    let m = archon_observability::ChannelMetrics::new();
    // Coerce into the trait object form — proves the impl is discoverable
    // at the new location.
    let _sink: &dyn ChannelMetricSink = &m;
    // Exercise the trait method through the dyn reference.
    let sink: &dyn ChannelMetricSink = &m;
    sink.record_sent();
    assert_eq!(m.snapshot().total_sent, 1);
}

// NOTE: The archon-core back-compat re-export (`pub use
// archon_observability::ChannelMetricSink;` in archon-core/src/metrics.rs)
// is pinned by crates/archon-tui/tests/channel_metrics_wiring_test.rs —
// see the first `use archon_core::ChannelMetricSink;` line there. Testing
// it from THIS file would require archon-observability to depend on
// archon-core, which is a cycle (archon-core already depends on
// archon-observability for the trait source). That wiring test lives in a
// crate that legitimately depends on both, so the contract stays covered
// without the cycle.

/// Prometheus exposition shape at the external-crate boundary: must
/// render all 5 metrics with correct Prometheus types. Full parsing +
/// type semantics are covered by the moved unit test in `src/metrics.rs`;
/// this is a minimal external-surface sanity check.
#[test]
fn format_prometheus_emits_five_metrics_at_external_boundary() {
    use archon_observability::format_prometheus;
    let m = archon_observability::ChannelMetrics::new();
    m.record_sent();
    m.record_drained(1);
    let body = format_prometheus(&m.snapshot());

    // Five metric families, each with `# HELP` + `# TYPE`.
    let help_count = body.matches("# HELP ").count();
    let type_count = body.matches("# TYPE ").count();
    assert_eq!(help_count, 5, "expected 5 HELP lines, got {help_count}:\n{body}");
    assert_eq!(type_count, 5, "expected 5 TYPE lines, got {type_count}:\n{body}");

    // max_batch_size must be `gauge`, not `counter` (regression guard —
    // high-water marks are gauges; `rate()` on them is nonsense).
    assert!(
        body.contains("# TYPE archon_tui_channel_max_batch_size gauge"),
        "max_batch_size must be gauge; body:\n{body}"
    );
    assert!(
        !body.contains("# TYPE archon_tui_channel_max_batch_size counter"),
        "max_batch_size must NOT be counter; body:\n{body}"
    );
}
