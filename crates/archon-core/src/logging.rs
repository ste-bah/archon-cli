//! # archon-core logging shim
//!
//! TASK-AGS-OBS-901-WIRE moved the owning definitions of the session-log
//! tracing wiring (`init_logging`, `LogGuard`, `LoggingError`) out of
//! this module into
//! [`archon_observability::file_init`](../../archon_observability/file_init/index.html).
//! The rewrite swapped the pre-LIFT `tracing_subscriber::fmt::layer()`
//! for a `RedactionLayer` mounted over the same non-blocking file
//! writer, closing the dead-wire where production session logs bypassed
//! secret redaction.
//!
//! This file retains two kinds of surface:
//!
//!   1. **Path utilities** — [`default_log_dir`] and [`rotate_logs`].
//!      These are filesystem helpers, not tracing-subscriber glue, so
//!      they stay in archon-core where `main.rs` + `setup.rs` already
//!      import them alongside the session-id routing logic.
//!
//!   2. **Back-compat re-exports** — `pub use archon_observability::
//!      {init_tracing_file as init_logging, LogGuard, LoggingError}`.
//!      Every existing `archon_core::logging::init_logging` caller
//!      (`src/main.rs:115`, `src/setup.rs:60`, `tests/logging_tests.rs`)
//!      compiles unchanged and now flows through the redaction path.
//!
//! If you are adding a NEW consumer, prefer
//! `archon_observability::init_tracing_file` directly.

use std::fs;
use std::path::{Path, PathBuf};

pub use archon_observability::{init_tracing_file as init_logging, LogGuard, LoggingError};

/// Default log directory: `~/.local/share/archon/logs/`.
pub fn default_log_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("archon")
        .join("logs")
}

/// Rotate log files in `log_dir`, keeping only the `max_files` most recent.
///
/// Files are sorted by modification time (oldest first). The oldest files
/// beyond `max_files` are deleted. Only `.log` files are considered.
///
/// Returns `Ok(())` if the directory doesn't exist or is empty.
pub fn rotate_logs(log_dir: &Path, max_files: u32) -> Result<(), LoggingError> {
    if !log_dir.exists() {
        return Ok(());
    }

    let mut log_files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "log") {
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            log_files.push((path, mtime));
        }
    }

    if log_files.len() <= max_files as usize {
        return Ok(());
    }

    // Sort oldest first
    log_files.sort_by_key(|(_, mtime)| *mtime);

    let to_remove = log_files.len() - max_files as usize;
    for (path, _) in log_files.iter().take(to_remove) {
        fs::remove_file(path)?;
    }

    Ok(())
}
