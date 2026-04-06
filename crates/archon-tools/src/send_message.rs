use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// SendMessageRequest — returned as JSON for the caller (agent loop) to deliver
// ---------------------------------------------------------------------------

/// A validated request to send a message to another agent.  The `SendMessageTool`
/// does not actually deliver the message — it validates parameters and produces
/// this struct so the outer agent loop can route the message to the target agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SendMessageRequest {
    pub agent_id: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SendMessageError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

// ---------------------------------------------------------------------------
// SendMessageTool — implements Tool
// ---------------------------------------------------------------------------

pub struct SendMessageTool;

impl SendMessageTool {
    fn validate_and_build(
        &self,
        input: &serde_json::Value,
    ) -> Result<SendMessageRequest, SendMessageError> {
        let agent_id = input
            .get("agent_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(SendMessageError::MissingField("agent_id"))?
            .to_string();

        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(SendMessageError::MissingField("message"))?
            .to_string();

        Ok(SendMessageRequest { agent_id, message })
    }
}

#[async_trait::async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
    }

    fn description(&self) -> &str {
        "Send a message to another agent by ID. Returns a SendMessageRequest \
         for the agent loop to deliver. The target agent receives the message \
         in its conversation."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["agent_id", "message"],
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The unique identifier of the target agent"
                },
                "message": {
                    "type": "string",
                    "description": "The message content to send to the target agent"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        match self.validate_and_build(&input) {
            Ok(request) => match serde_json::to_string_pretty(&request) {
                Ok(json_str) => ToolResult::success(json_str),
                Err(e) => ToolResult::error(format!("failed to serialize request: {e}")),
            },
            Err(e) => ToolResult::error(e.to_string()),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test-session".into(),
            mode: crate::tool::AgentMode::Normal,
            extra_dirs: vec![],
        }
    }

    #[tokio::test]
    async fn valid_input_returns_send_message_request() {
        let tool = SendMessageTool;
        let input = json!({
            "agent_id": "agent-42",
            "message": "Please review the parser module"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);

        let request: SendMessageRequest =
            serde_json::from_str(&result.content).expect("should deserialize");
        assert_eq!(request.agent_id, "agent-42");
        assert_eq!(request.message, "Please review the parser module");
    }

    #[tokio::test]
    async fn empty_agent_id_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "agent_id": "   ",
            "message": "hello"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("agent_id"),
            "error should mention 'agent_id': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "message": "hello"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("agent_id"),
            "error should mention 'agent_id': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn empty_message_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "agent_id": "agent-1",
            "message": ""
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("message"),
            "error should mention 'message': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn missing_message_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "agent_id": "agent-1"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("message"),
            "error should mention 'message': {}",
            result.content
        );
    }

    #[test]
    fn permission_level_is_risky() {
        let tool = SendMessageTool;
        assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Risky);
    }

    #[test]
    fn tool_metadata() {
        let tool = SendMessageTool;
        assert_eq!(tool.name(), "SendMessage");
        assert!(!tool.description().is_empty());

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("agent_id")));
        assert!(required.contains(&json!("message")));
    }

    #[tokio::test]
    async fn valid_request_round_trips_through_json() {
        let request = SendMessageRequest {
            agent_id: "test-agent".into(),
            message: "test message".into(),
        };

        let json_str = serde_json::to_string(&request).expect("serialize");
        let deserialized: SendMessageRequest =
            serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(request, deserialized);
    }
}
