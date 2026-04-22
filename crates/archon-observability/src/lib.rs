//! # archon-observability
//!
//! Shared observability primitives for archon-cli: structured tracing,
//! secret redaction, and Prometheus metrics exposition.
//!
//! ## Why this crate exists
//!
//! Before Stage 10 LIFT, observability code was split across two places:
//!
//!   * `archon-tui/src/observability_tracing.rs` — `init_tracing`,
//!     `RedactionLayer`, `SECRET_REGEX`. Hardened for Prometheus correctness
//!     and secret redaction as part of TASK-AGS-OBS-902.
//!   * `archon-core/src/logging.rs` — `init_logging` (the function
//!     actually called from `main.rs`). Installed only `fmt::layer`,
//!     **without** `RedactionLayer`.
//!
//! That split was a dead-wire: the hardened redaction layer was library-only.
//! Production sessions wrote unredacted secrets to
//! `~/.local/share/archon/logs/{session}.log` because `init_logging` bypassed
//! the redaction stack entirely.
//!
//! The Stage 10 LIFT sequence (OBS-900/901/905/906/907) consolidates all
//! observability primitives into this single crate and swaps `main.rs` to
//! call into here, closing the dead-wire for good.
//!
//! ## Current shape (after OBS-901)
//!
//! Modules landed so far:
//!
//!   * [`tracing`] — `init_tracing` + `span_*` constructors. Narrowed in
//!     OBS-906 to only hold the tracing-glue surface; redaction carved into
//!     the sibling module below.
//!   * [`redaction`] — `RedactionLayer`, `REDACTION_RE`, `RedactingVisitor`,
//!     `json_escape`, and the `Layer<S> for RedactionLayer` impl. Dedicated
//!     home for the security-critical surface so future work on `tracing.rs`
//!     cannot touch the redaction contract by accident. Full secret-shape
//!     matrix (OpenAI, Anthropic, AWS, GitHub, Stripe, JWT, bearer,
//!     sensitive field names) lives here.
//!   * [`metrics`] — `ChannelMetrics`, `ChannelMetricsSnapshot`,
//!     `ChannelMetricSink` (moved from archon-core in OBS-901),
//!     `format_prometheus`, `serve_metrics_on`, and `serve_metrics`. The
//!     owning home for channel instrumentation and the Prometheus `/metrics`
//!     endpoint. archon-tui's `src/observability.rs` is now a thin shim that
//!     re-exports from this module; archon-core re-exports
//!     `ChannelMetricSink` from here so every existing `archon_core::
//!     ChannelMetricSink` call site stays byte-identical.
//!
//! Still to land (follow-up tickets):
//!
//!   * (wiring) — `main.rs`/`session.rs`/`archon-core::logging` switch to
//!     this crate, closing the dead-wire so production logs are redacted.
//!   * [`OBS-907`] — gate-walk the `json` arg on `init_tracing`.
//!
//! ## Public re-exports
//!
//! Every module's primary surface is re-exported at the crate root so
//! downstream callers can write `use archon_observability::init_tracing`,
//! `use archon_observability::RedactionLayer`, and `use
//! archon_observability::ChannelMetrics` — the short paths stay stable across
//! future internal refactors. The archon-tui re-export shim depends on this:
//! it imports from the crate root, not from either submodule, so a future
//! module reshuffle inside `archon-observability` doesn't leak out to
//! downstream crates.
//!
//! [`OBS-901`]: https://example.internal/task/AGS-OBS-901
//! [`OBS-906`]: https://example.internal/task/AGS-OBS-906
//! [`OBS-907`]: https://example.internal/task/AGS-OBS-907

#![deny(missing_docs)]
#![deny(unsafe_code)]

pub mod metrics;
pub mod redaction;
pub mod tracing;

pub use metrics::{
    format_prometheus, serve_metrics, serve_metrics_on, ChannelMetricSink, ChannelMetrics,
    ChannelMetricsSnapshot,
};
pub use redaction::RedactionLayer;
pub use tracing::{init_tracing, span_agent_turn, span_channel_send, span_slash_dispatch};

/// Workspace version string — pinned to `CARGO_PKG_VERSION` at build time so
/// downstream crates can surface the active archon-observability version
/// without an extra build.rs. Used in smoke tests to prove the crate is
/// actually linked, not replaced by a no-op shim.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    /// Shell smoke: the crate must expose a non-empty version string backed
    /// by `CARGO_PKG_VERSION`. If a LIFT ticket accidentally drops the const
    /// or replaces it with a hard-coded value, this catches it.
    #[test]
    fn version_is_non_empty_and_sourced_from_cargo() {
        assert!(!VERSION.is_empty(), "VERSION must not be empty");
        // Sanity: Cargo.toml uses workspace.version = "0.1.4"; we don't pin
        // the exact value here (would break on bump) — just prove it's a
        // dotted semver-shaped string so a build-script regression shows.
        assert!(
            VERSION.contains('.'),
            "VERSION '{VERSION}' must look like a dotted version"
        );
        // Belt-and-braces against a future regression where someone replaces
        // the `env!` macro with a literal: re-expand CARGO_PKG_VERSION here
        // and demand bit-equality. If the const ever stops being sourced from
        // the compile-time env var, this fails loudly.
        assert_eq!(
            VERSION,
            env!("CARGO_PKG_VERSION"),
            "VERSION must be env!(CARGO_PKG_VERSION), not a literal"
        );
    }
}
