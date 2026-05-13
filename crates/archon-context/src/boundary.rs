use serde::{Deserialize, Serialize};

use crate::messages::ContextMessage;

/// Marker inserted at a compaction point to record what was removed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactBoundary {
    pub summary: String,
    pub tokens_removed: u64,
    pub tokens_remaining: u64,
    pub strategy: CompactionStrategy,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Which compaction strategy produced a boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionStrategy {
    Micro,
    Auto,
    Snip,
}

impl std::fmt::Display for CompactionStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Micro => write!(f, "Micro"),
            Self::Auto => write!(f, "Auto"),
            Self::Snip => write!(f, "Snip"),
        }
    }
}

impl CompactBoundary {
    /// Convert this boundary into a user-role [`ContextMessage`] for insertion
    /// into provider message history.
    pub fn to_message(&self) -> ContextMessage {
        let text = format!(
            "[Compaction Boundary — {} strategy] Removed {} tokens, {} tokens remaining. {}",
            self.strategy, self.tokens_removed, self.tokens_remaining, self.summary,
        );
        ContextMessage {
            role: "user".into(),
            content: serde_json::Value::String(text.clone()),
            estimated_tokens: (text.len() as f64 / 4.0).ceil() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_message_is_provider_safe_user_role() {
        let boundary = CompactBoundary {
            summary: "summary".into(),
            tokens_removed: 10,
            tokens_remaining: 20,
            strategy: CompactionStrategy::Micro,
            timestamp: chrono::Utc::now(),
        };

        assert_eq!(boundary.to_message().role, "user");
    }
}
