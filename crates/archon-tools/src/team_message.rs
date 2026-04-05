//! Team message types for TASK-CLI-312.

use serde::{Deserialize, Serialize};

/// Message type discriminator for routing and filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    Chat,
    TaskAssignment,
    StatusUpdate,
    Completion,
    Error,
}

/// A single message in the team inbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMessage {
    /// Sender's role name.
    pub from: String,
    /// Recipient's role name or "all".
    pub to: String,
    /// Message content (plain text or JSON string).
    pub content: String,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Message type for routing.
    pub message_type: MessageType,
}

impl TeamMessage {
    /// Create a simple chat message with the current timestamp.
    pub fn chat(
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            content: content.into(),
            timestamp: current_timestamp_ms(),
            message_type: MessageType::Chat,
        }
    }
}

fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
