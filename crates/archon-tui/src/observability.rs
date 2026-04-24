//! # `archon_tui::observability` — thin shim over `archon-observability`.
//!
//! TASK-AGS-OBS-901 lifted the owning definitions of [`ChannelMetrics`],
//! [`ChannelMetricsSnapshot`], [`format_prometheus`], [`serve_metrics_on`],
//! and [`serve_metrics`] out of this file into
//! [`archon_observability::metrics`]. OBS-905/906 had already lifted the
//! tracing-glue surface ([`init_tracing`], [`span_agent_turn`],
//! [`span_channel_send`], [`span_slash_dispatch`], [`RedactionLayer`]) to
//! `archon_observability::{tracing, redaction}`.
//!
//! Post-LIFT this file contains **only re-exports** so every existing
//! `archon_tui::observability::…` call site (state.rs, session.rs, unit +
//! integration tests, benches) compiles unchanged. The goal of this shim is
//! to hold the external surface stable during the wiring subtask that
//! follows OBS-901; once every caller is migrated to the
//! `archon_observability::` paths directly, this shim can be deleted.
//!
//! See `crates/archon-observability/src/metrics.rs` for the metrics impl +
//! unit tests. `observability_tracing.rs` remains as an OBS-905-era shim
//! that re-exports the tracing surface; it stays in place until the same
//! wiring subtask retires it alongside this file.
//!
//! **Do not add new code here.** New helpers go into
//! `archon-observability`.

pub use archon_observability::metrics::{
    ChannelMetrics, ChannelMetricsSnapshot, format_prometheus, serve_metrics, serve_metrics_on,
};

// OBS-905 / OBS-906 tracing surface. Re-exported from the in-tree
// observability_tracing shim so `archon_tui::observability::{init_tracing,
// span_*, RedactionLayer}` keeps resolving for every existing caller.
pub use crate::observability_tracing::{
    RedactionLayer, init_tracing, span_agent_turn, span_channel_send, span_slash_dispatch,
};
