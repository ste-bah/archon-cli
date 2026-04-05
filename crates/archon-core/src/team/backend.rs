//! Inbox backend trait and implementations for TASK-CLI-312.
//!
//! Three backends:
//! - `InMemoryBackend` — in-process channels; used in tests
//! - `FileBasedBackend` — JSONL files; default for production cross-process communication
//! - `RemoteBackend` — future placeholder stub

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use crate::team::message::{MessageType, TeamMessage};

// ---------------------------------------------------------------------------
// InboxBackend trait
// ---------------------------------------------------------------------------

/// Common interface for all inbox backends.
pub trait InboxBackend {
    /// Append a message to the named role's inbox.
    fn send(&mut self, role: &str, message: TeamMessage) -> Result<(), BackendError>;

    /// Read all pending messages for `role` and clear the inbox.
    ///
    /// Returns an empty `Vec` if no messages are waiting or the role is unknown.
    fn read_and_clear(&self, role: &str) -> Result<Vec<TeamMessage>, BackendError>;
}

/// Backend operation error.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Unknown role: {0}")]
    UnknownRole(String),
    #[error("mutex lock poisoned")]
    LockPoisoned,
}

// ---------------------------------------------------------------------------
// InMemoryBackend
// ---------------------------------------------------------------------------

/// In-process backend using `HashMap<role, Vec<TeamMessage>>`.
///
/// Used in unit tests. No file I/O.
#[derive(Debug, Default)]
pub struct InMemoryBackend {
    inboxes: std::sync::Mutex<HashMap<String, Vec<TeamMessage>>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-register a role so it appears in `send_to_all`.
    pub fn register(&self, role: &str) {
        let mut guard = self.inboxes.lock().unwrap_or_else(|e| e.into_inner());
        guard.entry(role.to_string()).or_default();
    }

    /// Broadcast a message to all registered roles.
    pub fn send_to_all(
        &self,
        from: &str,
        content: &str,
        message_type: MessageType,
    ) -> Result<(), BackendError> {
        let mut guard = self.inboxes.lock().map_err(|_| BackendError::LockPoisoned)?;
        let roles: Vec<String> = guard.keys().cloned().collect();
        for role in roles {
            let msg = TeamMessage {
                from: from.to_string(),
                to: role.clone(),
                content: content.to_string(),
                timestamp: 0,
                message_type: message_type.clone(),
            };
            guard.entry(role).or_default().push(msg);
        }
        Ok(())
    }
}

impl InboxBackend for InMemoryBackend {
    fn send(&mut self, role: &str, message: TeamMessage) -> Result<(), BackendError> {
        let mut guard = self.inboxes.lock().map_err(|_| BackendError::LockPoisoned)?;
        guard.entry(role.to_string()).or_default().push(message);
        Ok(())
    }

    fn read_and_clear(&self, role: &str) -> Result<Vec<TeamMessage>, BackendError> {
        let mut guard = self.inboxes.lock().map_err(|_| BackendError::LockPoisoned)?;
        Ok(guard.remove(role).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// FileBasedBackend
// ---------------------------------------------------------------------------

/// File-based backend using per-role JSONL inbox files.
///
/// Each role's inbox is `<dir>/inbox-<role>.jsonl`.
/// This backend works across processes sharing the same directory.
#[derive(Debug)]
pub struct FileBasedBackend {
    dir: PathBuf,
}

impl FileBasedBackend {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn inbox_path(&self, role: &str) -> PathBuf {
        self.dir.join(format!("inbox-{role}.jsonl"))
    }
}

impl InboxBackend for FileBasedBackend {
    fn send(&mut self, role: &str, message: TeamMessage) -> Result<(), BackendError> {
        let path = self.inbox_path(role);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let line = serde_json::to_string(&message)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    fn read_and_clear(&self, role: &str) -> Result<Vec<TeamMessage>, BackendError> {
        let path = self.inbox_path(role);
        if !path.exists() {
            return Ok(vec![]);
        }

        let content = std::fs::read_to_string(&path)?;
        let messages: Vec<TeamMessage> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        // Clear the file by truncating it
        std::fs::write(&path, "")?;

        Ok(messages)
    }
}

// ---------------------------------------------------------------------------
// RemoteBackend — stub for future extension
// ---------------------------------------------------------------------------

/// Placeholder for a future remote backend (e.g., Redis, HTTP).
#[derive(Debug, Default)]
pub struct RemoteBackend;

impl InboxBackend for RemoteBackend {
    fn send(&mut self, _role: &str, _message: TeamMessage) -> Result<(), BackendError> {
        Err(BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "RemoteBackend is not yet implemented",
        )))
    }

    fn read_and_clear(&self, _role: &str) -> Result<Vec<TeamMessage>, BackendError> {
        Err(BackendError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "RemoteBackend is not yet implemented",
        )))
    }
}
