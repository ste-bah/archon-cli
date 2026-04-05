//! Background session launcher.
//!
//! Spawns a new archon process in print mode with `setsid` (via
//! `process_group(0)`) so it survives the parent terminal closing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::storage::SessionError;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Information about a background session, persisted as `<id>.status.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundSessionInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub pid: Option<u32>,
    pub started_at: String,
    pub turns: u32,
    pub cost: f64,
    pub last_activity: Option<String>,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Directory helpers
// ---------------------------------------------------------------------------

/// Directory for background session files.
pub fn sessions_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archon")
        .join("sessions")
}

// ---------------------------------------------------------------------------
// Launch
// ---------------------------------------------------------------------------

/// Launch a background session using the default sessions directory.
///
/// Spawns a new archon process in print mode with `setsid`, returns session ID.
#[cfg(unix)]
pub fn launch_background(
    query: &str,
    name: Option<&str>,
    archon_binary: &Path,
) -> Result<String, SessionError> {
    launch_background_in_dir(&sessions_dir(), query, name, archon_binary)
}

/// Launch a background session in a specific directory (testable).
#[cfg(unix)]
pub fn launch_background_in_dir(
    dir: &Path,
    query: &str,
    name: Option<&str>,
    archon_binary: &Path,
) -> Result<String, SessionError> {
    use std::os::unix::process::CommandExt as _;
    use std::process::Command;

    let session_id = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(dir)?;

    let log_path = dir.join(format!("{session_id}.log"));
    let pid_path = dir.join(format!("{session_id}.pid"));
    let status_path = dir.join(format!("{session_id}.status.json"));

    // Write initial status
    let status = BackgroundSessionInfo {
        id: session_id.clone(),
        name: name.unwrap_or("").to_string(),
        status: "starting".to_string(),
        pid: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        turns: 0,
        cost: 0.0,
        last_activity: None,
        error: None,
    };
    let status_json = serde_json::to_string_pretty(&status)
        .map_err(|e| SessionError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    std::fs::write(&status_path, &status_json)?;

    // Create log file and spawn
    let log_file = std::fs::File::create(&log_path)?;
    let log_stderr = log_file.try_clone().map_err(|e| SessionError::IoError(e))?;

    let child = Command::new(archon_binary)
        .arg("-p")
        .arg(query)
        .arg("--output-format")
        .arg("stream-json")
        .stdin(std::process::Stdio::null())
        .stdout(log_file)
        .stderr(log_stderr)
        .process_group(0) // setsid equivalent on Unix
        .spawn()
        .map_err(SessionError::IoError)?;

    let pid = child.id();

    // Write PID file
    std::fs::write(&pid_path, pid.to_string())?;

    // Update status with PID
    let updated_status = BackgroundSessionInfo {
        pid: Some(pid),
        status: "running".to_string(),
        ..status
    };
    let updated_json = serde_json::to_string_pretty(&updated_status)
        .map_err(|e| SessionError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
    std::fs::write(&status_path, &updated_json)?;

    Ok(session_id)
}

// ---------------------------------------------------------------------------
// Kill
// ---------------------------------------------------------------------------

/// Kill a background session by ID using the default sessions directory.
#[cfg(unix)]
pub fn kill_session(session_id: &str) -> Result<(), SessionError> {
    kill_session_in_dir(&sessions_dir(), session_id)
}

/// Kill a background session by ID in a specific directory (testable).
#[cfg(unix)]
pub fn kill_session_in_dir(dir: &Path, session_id: &str) -> Result<(), SessionError> {
    let pid_path = dir.join(format!("{session_id}.pid"));
    let status_path = dir.join(format!("{session_id}.status.json"));

    // Try to read PID and send SIGTERM
    if pid_path.exists() {
        let pid_str = std::fs::read_to_string(&pid_path)?;
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            // Best-effort SIGTERM; ignore errors (process may already be dead)
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        // Remove PID file
        let _ = std::fs::remove_file(&pid_path);
    }

    // Update status to "killed"
    if status_path.exists() {
        let raw = std::fs::read_to_string(&status_path)?;
        if let Ok(mut info) = serde_json::from_str::<BackgroundSessionInfo>(&raw) {
            info.status = "killed".to_string();
            let json = serde_json::to_string_pretty(&info).map_err(|e| {
                SessionError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e))
            })?;
            std::fs::write(&status_path, json)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Liveness check
// ---------------------------------------------------------------------------

/// Check if a background session process is still running (default dir).
#[cfg(unix)]
pub fn is_session_alive(session_id: &str) -> bool {
    is_session_alive_in_dir(&sessions_dir(), session_id)
}

/// Check if a background session process is still running (custom dir).
#[cfg(unix)]
pub fn is_session_alive_in_dir(dir: &Path, session_id: &str) -> bool {
    let pid_path = dir.join(format!("{session_id}.pid"));
    if !pid_path.exists() {
        return false;
    }
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let pid: i32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    // kill(pid, 0) checks if process exists without sending a signal
    unsafe { libc::kill(pid, 0) == 0 }
}
