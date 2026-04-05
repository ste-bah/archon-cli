//! Log streaming for background sessions.
//!
//! Provides `stream_logs` (tail -f equivalent for running sessions) and
//! `view_logs` (non-streaming read).

use std::io::Write as _;
use std::path::Path;

use crate::background::sessions_dir;
use crate::storage::SessionError;

// ---------------------------------------------------------------------------
// View (non-streaming)
// ---------------------------------------------------------------------------

/// Read and return log file contents (non-streaming) from default dir.
pub fn view_logs(session_id: &str) -> Result<String, SessionError> {
    view_logs_in_dir(&sessions_dir(), session_id)
}

/// Read and return log file contents from a specific directory (testable).
pub fn view_logs_in_dir(dir: &Path, session_id: &str) -> Result<String, SessionError> {
    let log_path = dir.join(format!("{session_id}.log"));
    std::fs::read_to_string(&log_path).map_err(SessionError::IoError)
}

// ---------------------------------------------------------------------------
// Stream (tail -f)
// ---------------------------------------------------------------------------

/// Stream a session's log file to stdout.
///
/// If `follow` is true and the session is still alive, this watches the file
/// for new content (like `tail -f`). Otherwise it prints existing content and
/// returns.
#[cfg(unix)]
pub fn stream_logs(session_id: &str, follow: bool) -> Result<(), SessionError> {
    stream_logs_in_dir(&sessions_dir(), session_id, follow)
}

/// Stream logs from a specific directory (testable).
#[cfg(unix)]
pub fn stream_logs_in_dir(dir: &Path, session_id: &str, follow: bool) -> Result<(), SessionError> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let log_path = dir.join(format!("{session_id}.log"));
    let file = std::fs::File::open(&log_path).map_err(SessionError::IoError)?;
    let mut reader = BufReader::new(file);
    let mut stdout = std::io::stdout();

    // Read and print existing content
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).map_err(SessionError::IoError)?;
        if bytes_read == 0 {
            break;
        }
        let display_line = format_log_line(&line);
        let _ = stdout.write_all(display_line.as_bytes());
        let _ = stdout.flush();
    }

    if !follow {
        return Ok(());
    }

    // Follow mode: poll for new content while the session is alive
    loop {
        if !crate::background::is_session_alive_in_dir(dir, session_id) {
            // Read any remaining content
            loop {
                let mut line = String::new();
                let bytes_read = reader.read_line(&mut line).map_err(SessionError::IoError)?;
                if bytes_read == 0 {
                    break;
                }
                let display_line = format_log_line(&line);
                let _ = stdout.write_all(display_line.as_bytes());
                let _ = stdout.flush();
            }
            break;
        }

        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line).map_err(SessionError::IoError)?;
        if bytes_read == 0 {
            // No new data yet; sleep briefly and retry
            std::thread::sleep(std::time::Duration::from_millis(200));

            // Re-seek to handle file truncation/rotation
            let pos = reader.stream_position().map_err(SessionError::IoError)?;
            let meta = std::fs::metadata(&log_path).map_err(SessionError::IoError)?;
            if meta.len() < pos {
                reader
                    .seek(SeekFrom::Start(0))
                    .map_err(SessionError::IoError)?;
            }
            continue;
        }
        let display_line = format_log_line(&line);
        let _ = stdout.write_all(display_line.as_bytes());
        let _ = stdout.flush();
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Format a single NDJSON log line for display.
///
/// For assistant text events, the text is shown as-is. Tool call events are
/// rendered as a one-liner summary. Unparseable lines pass through unchanged.
fn format_log_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Try to parse as JSON
    let value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return line.to_string(),
    };

    // Handle stream-json events
    match value.get("type").and_then(|t| t.as_str()) {
        Some("assistant" | "text" | "content_block_delta") => {
            if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
                return text.to_string();
            }
            if let Some(delta) = value.get("delta")
                && let Some(text) = delta.get("text").and_then(|t| t.as_str())
            {
                return text.to_string();
            }
            line.to_string()
        }
        Some("tool_use" | "tool_call") => {
            let name = value
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            format!("[tool: {name}]\n")
        }
        Some("tool_result") => {
            let is_error = value
                .get("is_error")
                .and_then(|e| e.as_bool())
                .unwrap_or(false);
            if is_error {
                "[tool result: ERROR]\n".to_string()
            } else {
                "[tool result: OK]\n".to_string()
            }
        }
        _ => line.to_string(),
    }
}
