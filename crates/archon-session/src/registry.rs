//! Session registry: list, get, and clean up background sessions.

use std::path::Path;

use crate::background::{BackgroundSessionInfo, sessions_dir};
use crate::storage::SessionError;

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// List all background sessions (running + completed) from default dir.
pub fn list_sessions() -> Result<Vec<BackgroundSessionInfo>, SessionError> {
    list_sessions_in_dir(&sessions_dir())
}

/// List all background sessions in a specific directory (testable).
pub fn list_sessions_in_dir(dir: &Path) -> Result<Vec<BackgroundSessionInfo>, SessionError> {
    let mut sessions = Vec::new();

    if !dir.exists() {
        return Ok(sessions);
    }

    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.ends_with(".status.json") {
            continue;
        }

        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let info: BackgroundSessionInfo = match serde_json::from_str(&raw) {
            Ok(i) => i,
            Err(_) => continue,
        };

        // Check liveness for sessions marked as running
        #[cfg(unix)]
        let info = {
            let mut i = info;
            if i.status == "running" && !crate::background::is_session_alive_in_dir(dir, &i.id) {
                i.status = "stale".to_string();
            }
            i
        };

        sessions.push(info);
    }

    // Sort by start time (newest first)
    sessions.sort_by(|a, b| b.started_at.cmp(&a.started_at));

    Ok(sessions)
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

/// Get info for a single session from default dir.
pub fn get_session(id: &str) -> Result<BackgroundSessionInfo, SessionError> {
    get_session_in_dir(&sessions_dir(), id)
}

/// Get info for a single session in a specific directory (testable).
pub fn get_session_in_dir(dir: &Path, id: &str) -> Result<BackgroundSessionInfo, SessionError> {
    let status_path = dir.join(format!("{id}.status.json"));
    if !status_path.exists() {
        return Err(SessionError::NotFound(format!(
            "background session {id} not found"
        )));
    }
    let raw = std::fs::read_to_string(&status_path)?;
    let info: BackgroundSessionInfo = serde_json::from_str(&raw)
        .map_err(|e| SessionError::DbError(format!("failed to parse status file for {id}: {e}")))?;
    Ok(info)
}

// ---------------------------------------------------------------------------
// Cleanup: old sessions
// ---------------------------------------------------------------------------

/// Clean up old completed sessions (older than `max_age_days`) from default dir.
pub fn cleanup_old_sessions(max_age_days: u64) -> Result<usize, SessionError> {
    cleanup_old_sessions_in_dir(&sessions_dir(), max_age_days)
}

/// Clean up old completed sessions in a specific directory (testable).
pub fn cleanup_old_sessions_in_dir(dir: &Path, max_age_days: u64) -> Result<usize, SessionError> {
    if !dir.exists() {
        return Ok(0);
    }

    let cutoff = chrono::Utc::now() - chrono::Duration::days(max_age_days as i64);
    let cutoff_str = cutoff.to_rfc3339();
    let mut removed = 0;

    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.ends_with(".status.json") {
            continue;
        }

        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let info: BackgroundSessionInfo = match serde_json::from_str(&raw) {
            Ok(i) => i,
            Err(_) => continue,
        };

        // Only clean completed/error/killed sessions that are old enough
        let cleanable = matches!(
            info.status.as_str(),
            "completed" | "error" | "killed" | "stale"
        );
        if cleanable && info.started_at < cutoff_str {
            // Remove all files for this session
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(dir.join(format!("{}.log", info.id)));
            let _ = std::fs::remove_file(dir.join(format!("{}.pid", info.id)));
            removed += 1;
        }
    }

    Ok(removed)
}

// ---------------------------------------------------------------------------
// Cleanup: stale PIDs
// ---------------------------------------------------------------------------

/// Detect and clean stale PID files from default dir.
pub fn cleanup_stale_pids() -> Result<usize, SessionError> {
    cleanup_stale_pids_in_dir(&sessions_dir())
}

/// Detect and clean stale PID files in a specific directory (testable).
pub fn cleanup_stale_pids_in_dir(dir: &Path) -> Result<usize, SessionError> {
    if !dir.exists() {
        return Ok(0);
    }

    let mut cleaned = 0;

    let entries = std::fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if !name.ends_with(".status.json") {
            continue;
        }

        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut info: BackgroundSessionInfo = match serde_json::from_str(&raw) {
            Ok(i) => i,
            Err(_) => continue,
        };

        if info.status != "running" {
            continue;
        }

        // Check if the process is still alive
        #[cfg(unix)]
        let alive = crate::background::is_session_alive_in_dir(dir, &info.id);
        #[cfg(not(unix))]
        let alive = false;

        if !alive {
            info.status = "stale".to_string();
            let json = serde_json::to_string_pretty(&info).map_err(|e| {
                SessionError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e))
            })?;
            std::fs::write(&path, json)?;
            // Remove the stale PID file
            let _ = std::fs::remove_file(dir.join(format!("{}.pid", info.id)));
            cleaned += 1;
        }
    }

    Ok(cleaned)
}
