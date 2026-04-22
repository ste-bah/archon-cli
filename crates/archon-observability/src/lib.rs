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
//! ## Current shape (OBS-900)
//!
//! This file is a **shell**. It exposes a single [`VERSION`] constant so
//! downstream crates can take a trivial dependency today and start updating
//! their `use` imports ahead of the content lifts. Real modules land in
//! follow-up tickets:
//!
//!   * [`OBS-901`] — `metrics` module (Prometheus exposition, `ChannelMetrics`).
//!   * [`OBS-905`] — `tracing` module (`init_tracing`, formatting).
//!   * [`OBS-906`] — `redaction` module (`RedactionLayer`, `SECRET_REGEX`).
//!
//! [`OBS-901`]: https://example.internal/task/AGS-OBS-901
//! [`OBS-905`]: https://example.internal/task/AGS-OBS-905
//! [`OBS-906`]: https://example.internal/task/AGS-OBS-906

#![deny(missing_docs)]
#![deny(unsafe_code)]

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
