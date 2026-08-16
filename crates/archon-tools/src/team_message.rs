//! Team message types (TASK-CLI-312), wired as the member envelope in #184 M5.
//!
//! These types existed with nothing producing or consuming them. What they were
//! missing was the thing they describe: a message from one member to another,
//! attributed. `route_text` enqueued the raw string, so a member receiving
//! "please review src/foo.rs" had no idea who sent it and could not reply.
//!
//! That is what a team envelope is for, so the types become it rather than
//! being replaced.

use serde::{Deserialize, Serialize};

/// Message type discriminator for routing and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    Chat,
    TaskAssignment,
    StatusUpdate,
    Completion,
    Error,
}

impl MessageType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::TaskAssignment => "task_assignment",
            Self::StatusUpdate => "status_update",
            Self::Completion => "completion",
            Self::Error => "error",
        }
    }
}

/// One message from a team member to another.
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
    /// A message stamped with the current time.
    pub fn now(
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
        message_type: MessageType,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            content: content.into(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            message_type,
        }
    }

    /// What the recipient actually reads.
    ///
    /// Attribution is the whole point — a member that cannot tell who asked
    /// cannot answer — so `from` is in the frame rather than folded into the
    /// text, where a model would have to parse it back out.
    ///
    /// Escaped with the same rule as the other envelopes: the content is
    /// model-authored, and an unescaped `"` in it would otherwise close the
    /// attribute and let the sender forge a different `from`.
    pub fn render(&self) -> String {
        format!(
            "<archon_team_message from=\"{}\" to=\"{}\" type=\"{}\">\n{}\n</archon_team_message>",
            crate::send_message::xml_escape(&self.from),
            crate::send_message::xml_escape(&self.to),
            self.message_type.as_str(),
            crate::send_message::xml_escape(&self.content),
        )
    }
}

#[cfg(test)]
#[path = "team_message_tests.rs"]
mod tests;
