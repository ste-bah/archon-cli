use std::fs;
use std::path::{Path, PathBuf};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("logging I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("logging setup error: {0}")]
    SetupError(String),
}

#[cfg(unix)]
fn secure_file_permissions(path: &std::path::Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

// ---------------------------------------------------------------------------
// Guard -- must be held for the lifetime of the application
// ---------------------------------------------------------------------------

/// Holds the tracing guard. Drop this to flush logs.
pub struct LogGuard {
    _worker_guard: tracing_appender::non_blocking::WorkerGuard,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Default log directory: `~/.local/share/archon/logs/`
pub fn default_log_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("archon")
        .join("logs")
}

/// Initialize the tracing logging system.
///
/// - Creates `log_dir` if it does not exist.
/// - Opens `{session_id}.log` in `log_dir` for appending.
/// - Configures tracing with the given level (overridden by `ARCHON_LOG` env).
/// - Logs ONLY to the file (no stderr/stdout to avoid corrupting TUI).
///
/// Returns a `LogGuard` that MUST be held until application exit.
pub fn init_logging(
    session_id: &str,
    config_level: &str,
    log_dir: &Path,
) -> Result<LogGuard, LoggingError> {
    // Ensure log directory exists
    fs::create_dir_all(log_dir)?;

    // Open log file for appending
    let log_path = log_dir.join(format!("{session_id}.log"));
    let log_file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    #[cfg(unix)]
    secure_file_permissions(&log_path)?;

    // Non-blocking writer for the file
    let (file_writer, guard) = tracing_appender::non_blocking(log_file);

    // Build the env filter: ARCHON_LOG env var takes precedence over config.
    // Always suppress noisy third-party crate logs regardless of base level.
    let base_filter = EnvFilter::try_from_env("ARCHON_LOG").unwrap_or_else(|_| {
        EnvFilter::try_new(config_level).unwrap_or_else(|_| EnvFilter::new("info"))
    });
    let filter = base_filter
        .add_directive("cozo_ce=warn".parse().expect("valid directive"))
        .add_directive("cozo=warn".parse().expect("valid directive"))
        .add_directive("hyper_util=warn".parse().expect("valid directive"))
        .add_directive("reqwest=warn".parse().expect("valid directive"));

    // File layer: timestamp + level + module + message
    // This is the ONLY output target — no stderr/stdout layer so the TUI is not
    // corrupted by log lines.
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer.with_max_level(tracing::Level::TRACE))
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .try_init()
        .map_err(|e| LoggingError::SetupError(format!("failed to init tracing: {e}")))?;

    Ok(LogGuard {
        _worker_guard: guard,
    })
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
