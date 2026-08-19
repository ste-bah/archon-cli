//! File-backed session logging with `RedactionLayer` as the sole emitter.
//!
//! TASK-AGS-OBS-901-WIRE closes the dead-wire where the pre-LIFT
//! production path (`archon_core::logging::init_logging`) wrote session
//! logs via a plain `tracing_subscriber::fmt` layer — meaning every
//! secret that passed through a `tracing::info!(api_key = ...)` call
//! landed unredacted in `~/.local/share/archon/logs/{session}.log`.
//!
//! This module provides the replacement: `init_tracing_file(session_id,
//! log_level, log_dir)` installs a global subscriber whose SOLE event
//! emitter is a `RedactionLayer` wrapping a non-blocking file writer.
//! The `archon-core` call site (`crates/archon-core/src/logging.rs`)
//! re-exports this function as `init_logging`, so every existing
//! caller of `archon_core::logging::init_logging` now routes through
//! the redaction path without any signature change.
//!
//! # Security architecture — parallel-sinks tombstone
//!
//! Tracing subscriber layers are **parallel sinks**, not filters. If we
//! stacked a `tracing_subscriber::fmt::layer()` alongside
//! `RedactionLayer`, every event would be written TWICE — once by the
//! redaction layer (secrets scrubbed) and once by the fmt layer (raw
//! secrets preserved). That would be a catastrophic secret leak.
//!
//! The defence is the same one documented at the top of `src/tracing.rs`
//! and `src/redaction.rs`: **`RedactionLayer` is the only layer that
//! emits events**. No `fmt::layer()`. No `bunyan`. No `tracing-tree`.
//! The subscriber stack is exactly `registry + EnvFilter + RedactionLayer`,
//! and the `RedactionLayer` writer is a non-blocking handle wrapping
//! the file descriptor.
//!
//! # Behavioural contract preserved from pre-LIFT `init_logging`
//!
//!   * Creates `log_dir` if missing (`create_dir_all`).
//!   * Opens `{session_id}.log` in append mode.
//!   * Sets 0600 permissions on Unix (secret posture).
//!   * Returns a `LogGuard` holding the `tracing_appender::WorkerGuard`
//!     that MUST be held for the lifetime of the application — drop
//!     flushes the queued events.
//!   * Applies the same third-party noise filters (`cozo=warn`,
//!     `cozo_ce=warn`, `hyper_util=warn`, `reqwest=warn`) as the
//!     pre-LIFT function.
//!   * `EnvFilter` source: `ARCHON_LOG` takes precedence, then `RUST_LOG`,
//!     then the `log_level` arg, then `"info"` as the final fallback.
//!     `RUST_LOG` was added after it turned out to be ignored outright —
//!     see `session_filter`.
//!
//! # Behavioural delta from pre-LIFT `init_logging`
//!
//!   * **Line format change.** Pre-LIFT format was
//!     `<RFC3339-timestamp> <LEVEL> <target>: <message> <fields>` via
//!     `tracing_subscriber::fmt::layer()`. Post-LIFT format is the
//!     `RedactionLayer` pretty format:
//!     `[<LEVEL> <target>]{span1::span2} name=value name=value`. There
//!     is no per-line timestamp — session filenames already carry
//!     wall-clock ordering via their mtime, and a future ticket can
//!     add an optional timestamp flag to `RedactionLayer` if downstream
//!     tooling needs one. Grep/awk scripts that rely on the pre-LIFT
//!     timestamp column WILL need to be updated.

use std::fs;
use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::redaction::RedactionLayer;

/// Error surface for `init_tracing_file`. Matches the pre-LIFT
/// `archon_core::logging::LoggingError` variants so the `pub use`
/// re-export in `archon-core` preserves every call-site match arm.
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    /// I/O error from creating `log_dir`, opening the session log file, or
    /// (on Unix) chmod'ing it to 0600. Wraps the underlying `std::io::Error`
    /// verbatim so call sites that pattern-match on `io::ErrorKind` keep
    /// working against the re-export shim in `archon_core::logging`.
    #[error("logging I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Subscriber install failed — typically `try_init` rejecting the call
    /// because a global default is already installed in this process. Fatal
    /// for session-scoped logging because the earlier install's writer is
    /// unreachable from here, so we surface the reason rather than swallow.
    #[error("logging setup error: {0}")]
    SetupError(String),
}

/// Guard holding the `tracing_appender` non-blocking worker. Drop flushes
/// the queued events to the underlying file handle. MUST be held until
/// application exit — dropping early causes the tail of the log to be
/// lost. The `_worker_guard` field is private so callers cannot
/// destructure and drop it by accident.
pub struct LogGuard {
    _worker_guard: WorkerGuard,
}

#[cfg(unix)]
fn secure_file_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

/// The filter for a session log: `ARCHON_LOG`, then `RUST_LOG`, then the
/// caller's level, then `info`.
///
/// `RUST_LOG` is honoured because it is the universal Rust convention and the
/// first thing anyone reaches for. Without it this silently ignored
/// `RUST_LOG=archon_llm=debug` and fell back to the caller's level — `info` by
/// default — so every debug event was filtered out and the session log looked
/// empty. Nothing failed and nothing warned; the events simply never reached
/// the writer, which is indistinguishable from logging being broken.
///
/// Noise filters for third-party crates are layered on top regardless, so
/// `debug` at the root does not drown the log in cozo/hyper/reqwest chatter.
fn session_filter(log_level: &str) -> EnvFilter {
    session_filter_from(
        std::env::var("ARCHON_LOG").ok().as_deref(),
        std::env::var("RUST_LOG").ok().as_deref(),
        log_level,
    )
}

/// The precedence itself, with the environment passed in.
///
/// Split out so the tests can exercise every branch without mutating
/// process-wide state — which this crate could not do anyway, since it denies
/// `unsafe` and `set_var` is unsafe as of the 2024 edition.
fn session_filter_from(
    archon_log: Option<&str>,
    rust_log: Option<&str>,
    log_level: &str,
) -> EnvFilter {
    let base = archon_log
        .and_then(|value| EnvFilter::try_new(value).ok())
        .or_else(|| rust_log.and_then(|value| EnvFilter::try_new(value).ok()))
        .unwrap_or_else(|| {
            EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info"))
        });

    base.add_directive("cozo_ce=warn".parse().expect("valid directive"))
        .add_directive("cozo=warn".parse().expect("valid directive"))
        .add_directive("hyper_util=warn".parse().expect("valid directive"))
        .add_directive("reqwest=warn".parse().expect("valid directive"))
}

/// Install the global tracing subscriber with `RedactionLayer` as the
/// SOLE event emitter, writing to `{session_id}.log` inside `log_dir`.
///
/// See the module-level docs for the full behavioural contract and the
/// security rationale behind the single-sink architecture.
///
/// # Errors
///
/// Returns `LoggingError::IoError` if the log directory cannot be
/// created, the log file cannot be opened, or (on Unix) the permission
/// bits cannot be set. Returns `LoggingError::SetupError` if the
/// `tracing_subscriber` registry refuses to install — this is normally
/// the "global default has already been set" path from an earlier call
/// in the same process, which is fatal for session-scoped logging
/// because the earlier call's writer is unreachable from here.
pub fn init_tracing_file(
    session_id: &str,
    log_level: &str,
    log_dir: &Path,
) -> Result<LogGuard, LoggingError> {
    init_tracing_file_with_otlp(session_id, log_level, log_dir, None)
}

/// As [`init_tracing_file`], plus OTLP span export when an endpoint is
/// configured (#189 Phase 10).
///
/// This is the function the binary actually reaches — `init_tracing` in the
/// sibling module is the stderr path. The endpoint has to be plumbed through
/// *here* or the setting would be inert in the one place it matters.
///
/// The export is attached to the `RedactionLayer` rather than stacked beside
/// it, for the same reason the module header gives for refusing a `fmt` layer:
/// tracing layers are parallel sinks, so a second one would ship the
/// unredacted copy — and off the machine this time, not just to a local file.
pub fn init_tracing_file_with_otlp(
    session_id: &str,
    log_level: &str,
    log_dir: &Path,
    otlp_endpoint: Option<&str>,
) -> Result<LogGuard, LoggingError> {
    fs::create_dir_all(log_dir)?;

    let log_path = log_dir.join(format!("{session_id}.log"));
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    #[cfg(unix)]
    secure_file_permissions(&log_path)?;

    // Non-blocking writer so tracing emits never stall the hot path on
    // disk I/O. The returned WorkerGuard is handed to the caller via
    // LogGuard and MUST outlive the process.
    let (file_writer, guard) = tracing_appender::non_blocking(log_file);

    let filter = session_filter(log_level);

    // RedactionLayer is the SOLE emitter. See module-level security
    // architecture comment for the parallel-sinks tombstone rationale.
    let mut redaction = RedactionLayer::with_writer(file_writer);
    if let Some(endpoint) = otlp_endpoint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // A configured endpoint that will not build is logged and dropped:
        // telemetry going nowhere is a degraded run, not a reason to refuse to
        // start a session the user asked for.
        match crate::otlp::OtlpExport::new(endpoint, "archon") {
            Ok(export) => {
                let export = std::sync::Arc::new(export);
                crate::otlp::set_global(&export);
                redaction = redaction.with_otlp(export);
            }
            Err(error) => {
                eprintln!("warning: OTLP export to {endpoint} could not start: {error}");
            }
        }
    }

    tracing_subscriber::registry()
        .with(filter)
        .with(redaction)
        .try_init()
        .map_err(|e| LoggingError::SetupError(format!("failed to init tracing: {e}")))?;

    Ok(LogGuard {
        _worker_guard: guard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression. `RUST_LOG` was ignored outright, so the one knob
    /// everybody reaches for did nothing and the session log looked empty.
    #[test]
    fn rust_log_reaches_the_session_filter() {
        let rendered = session_filter_from(None, Some("archon_llm=debug"), "info").to_string();

        assert!(
            rendered.contains("archon_llm=debug"),
            "RUST_LOG must reach the filter, got: {rendered}"
        );
    }

    /// `ARCHON_LOG` stays the more specific knob and keeps winning.
    #[test]
    fn archon_log_still_takes_precedence_over_rust_log() {
        let rendered =
            session_filter_from(Some("archon_core=trace"), Some("archon_llm=debug"), "info")
                .to_string();

        assert!(rendered.contains("archon_core=trace"), "{rendered}");
        assert!(!rendered.contains("archon_llm=debug"), "{rendered}");
    }

    #[test]
    fn the_callers_level_applies_when_neither_is_set() {
        let rendered = session_filter_from(None, None, "debug").to_string();

        assert!(rendered.contains("debug"), "{rendered}");
    }

    /// A malformed value must fall through to the next source rather than
    /// taking the whole filter down to the default.
    #[test]
    fn an_unparseable_value_falls_through() {
        let rendered =
            session_filter_from(Some("=@!not a filter="), Some("archon_llm=debug"), "info")
                .to_string();

        assert!(rendered.contains("archon_llm=debug"), "{rendered}");
    }

    /// The noise filters must survive whichever source won, or a root-level
    /// `debug` buries the session log in third-party chatter.
    #[test]
    fn third_party_noise_filters_are_always_applied() {
        let rendered = session_filter_from(None, Some("debug"), "info").to_string();

        for directive in [
            "cozo=warn",
            "cozo_ce=warn",
            "hyper_util=warn",
            "reqwest=warn",
        ] {
            assert!(
                rendered.contains(directive),
                "missing {directive}: {rendered}"
            );
        }
    }
}
