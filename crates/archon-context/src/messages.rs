use serde::{Deserialize, Serialize};

/// A conversation message for context management.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMessage {
    pub role: String,
    pub content: serde_json::Value,
    /// Estimated token count for this message.
    pub estimated_tokens: u64,
}

impl ContextMessage {
    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: serde_json::Value::String(content.into()),
            estimated_tokens: (content.len() as f64 / 4.0).ceil() as u64,
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: "assistant".into(),
            content: serde_json::Value::String(content.into()),
            estimated_tokens: (content.len() as f64 / 4.0).ceil() as u64,
        }
    }
}

/// Total estimated tokens across a message list.
pub fn total_estimated_tokens(messages: &[ContextMessage]) -> u64 {
    messages.iter().map(|m| m.estimated_tokens).sum()
}

/// Format token usage for display.
pub fn format_usage(input: u64, output: u64, cache: u64) -> String {
    format!(
        "in:{}k out:{}k cache:{}k",
        input / 1000,
        output / 1000,
        cache / 1000,
    )
}
