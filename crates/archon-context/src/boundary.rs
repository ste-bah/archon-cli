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
    /// Convert this boundary into a system-role [`ContextMessage`] for insertion
    /// into the conversation history.
    pub fn to_message(&self) -> ContextMessage {
        let text = format!(
            "[Compaction Boundary — {} strategy] Removed {} tokens, {} tokens remaining. {}",
            self.strategy, self.tokens_removed, self.tokens_remaining, self.summary,
        );
        ContextMessage {
            role: "system".into(),
            content: serde_json::Value::String(text.clone()),
            estimated_tokens: (text.len() as f64 / 4.0).ceil() as u64,
        }
    }
}
