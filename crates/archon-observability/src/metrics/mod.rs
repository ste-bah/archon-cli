//! Channel observability instrumentation for AgentEvent channel.
//!
//! Lifted from `crates/archon-tui/src/observability.rs` in TASK-AGS-OBS-901
//! as the final step of the Stage 10 LIFT sequence — consolidates every
//! observability primitive (tracing, redaction, metrics) into this crate
//! so archon-core's dead-wire can be closed by a single subscriber
//! install point.
//!
//! Content is verbatim from the pre-LIFT `archon-tui/src/observability.rs`
//! EXCEPT for one necessary shape change: the `ChannelMetricSink` trait
//! moved out of `archon-core/src/metrics.rs` into this file. That move
//! avoids an orphan-rule cycle: the impl `ChannelMetricSink for
//! ChannelMetrics` requires trait + type to live together, and making
//! archon-observability depend on archon-core (to pull the trait) plus a
//! future edge archon-core → archon-observability (for `init_tracing`)
//! would close a cycle. archon-core now re-exports the trait from here
//! via a single-line `pub use` shim so every existing `archon_core::
//! ChannelMetricSink` call site stays byte-identical.
//!
//! # Module layout (OBS-SPLIT-METRICS)
//!
//! The original single-file `metrics.rs` was split for readability:
//!
//!   * [`channel`] — the metrics data model: [`ChannelMetrics`],
//!     [`ChannelMetricsSnapshot`], and [`format_prometheus`].
//!   * [`server`] — the `/metrics` HTTP exposition surface:
//!     [`serve_metrics`] and [`serve_metrics_on`].
//!
//! The [`ChannelMetricSink`] trait lives in this `mod.rs` because the
//! `impl ChannelMetricSink for ChannelMetrics` block in `channel.rs` must
//! co-locate with the trait in the same crate (orphan rule) and putting
//! the trait here lets both `channel.rs` and any future sibling module
//! reference it via `super::ChannelMetricSink` without a circular path.
//!
//! # Metrics surface
//!
//!   * [`ChannelMetrics`] — the live, atomic/histogram counter set.
//!   * [`ChannelMetricsSnapshot`] — immutable moment-in-time view.
//!   * [`ChannelMetricSink`] — trait for any component that observes
//!     channel send/drain events (moved in from archon-core).
//!   * [`format_prometheus`] — renders a snapshot as Prometheus
//!     text-exposition v0.0.4.
//!   * [`serve_metrics_on`] / [`serve_metrics`] — HTTP serving of the
//!     `/metrics` endpoint. Binds to `127.0.0.1` only (no TLS, no auth
//!     per the original spec).

/// Trait for receiving channel instrumentation events.
///
/// Implementors must be `Send + Sync` (required for `Arc<dyn ChannelMetricSink>`).
///
/// OBS-901 note: moved here from `archon-core/src/metrics.rs` to co-locate
/// trait + type + impl in one crate. archon-core keeps a `pub use` shim so
/// `archon_core::ChannelMetricSink` still resolves.
pub trait ChannelMetricSink: Send + Sync {
    /// Called when a message is successfully sent into the channel.
    fn record_sent(&self);
    /// Called when a batch of messages is drained from the channel.
    /// `batch_size` is the number of messages in the drain batch.
    fn record_drained(&self, batch_size: u64);
}

pub mod channel;
pub mod server;

pub use channel::{format_prometheus, ChannelMetrics, ChannelMetricsSnapshot};
pub use server::{serve_metrics, serve_metrics_on};
