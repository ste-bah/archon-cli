use serde::{Deserialize, Serialize};

/// Messages exchanged between local client and remote headless agent
/// over the SSH channel stdin/stdout as JSON-lines.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    UserMessage { content: String },
    AssistantMessage { content: String },
    ToolCall { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
    Event { kind: String, data: serde_json::Value },
    Error { message: String },
    Ping,
    Pong,
}

impl AgentMessage {
    /// Serialize to a JSON line (JSON object + newline).
    pub fn to_json_line(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string(self)? + "\n")
    }

    /// Deserialize from a JSON line, trimming surrounding whitespace first.
    pub fn from_json_line(line: &str) -> anyhow::Result<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            anyhow::bail!("cannot deserialize AgentMessage from empty input");
        }
        Ok(serde_json::from_str(trimmed)?)
    }
}
