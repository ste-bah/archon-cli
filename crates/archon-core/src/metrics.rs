//! # `ChannelMetricSink` back-compat shim.
//!
//! TASK-AGS-OBS-901 moved the owning definition of `ChannelMetricSink` from
//! this file into `archon-observability` so the trait, its sole production
//! implementor (`ChannelMetrics`), and the `impl ChannelMetricSink for
//! ChannelMetrics` block could all live in one crate without tripping the
//! Rust orphan rule. Without the move the LIFT would have had to either
//! keep two files in sync forever or add a second cross-crate impl path —
//! both worse than a single `pub use` shim.
//!
//! This file retains **one line of surface**: a re-export of the trait so
//! every existing `archon_core::ChannelMetricSink` call site (agent.rs,
//! archon-tui tests, the wiring integration tests) continues to resolve
//! byte-identically. The `lib.rs` `pub use metrics::ChannelMetricSink`
//! re-export means `archon_core::ChannelMetricSink` is also still reachable
//! at the short path.
//!
//! If you are adding a NEW consumer, prefer
//! `archon_observability::ChannelMetricSink` — this shim exists only for
//! back-compat with the pre-OBS-901 call sites.

pub use archon_observability::ChannelMetricSink;
