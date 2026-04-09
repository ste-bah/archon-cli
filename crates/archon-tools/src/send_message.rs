use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// SendMessageRequest — returned as JSON for the caller (agent loop) to deliver
// ---------------------------------------------------------------------------

/// A validated request to send a message to another agent.  The `SendMessageTool`
/// does not actually deliver the message — it validates parameters and produces
/// this struct so the outer agent loop can route the message to the target agent.
///
/// Fields match Claude Code's SendMessage schema:
/// - `to`: agent name (from name registry) or raw agent ID
/// - `message`: plain text message content
/// - `summary`: short preview for UI — schema-optional but validation-required
///   for string messages (source: SendMessageTool.ts lines 667-674)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SendMessageRequest {
    /// Target: agent name or agent ID.
    pub to: String,
    /// Message content to deliver.
    pub message: String,
    /// Short preview for UI. Schema-optional, validated as required for string msgs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Agent ID validation
// ---------------------------------------------------------------------------

/// Check if a string looks like a valid agent ID (not arbitrary text).
///
/// Agent IDs are UUIDs or structured IDs like "agent-<uuid>".
/// Rejects: empty, contains spaces, longer than 128 chars.
pub fn is_valid_agent_id(s: &str) -> bool {
    !s.is_empty() && !s.contains(' ') && s.len() <= 128
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
        ctx: &ToolContext,
    ) -> Result<SendMessageRequest, SendMessageError> {
        // --- Extract and validate `to` ---
        let to = input
            .get("to")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(SendMessageError::MissingField("to"))?
            .trim()
            .to_string();

        // Early guard: reject broadcast target "*"
        if to == "*" {
            return Err(SendMessageError::InvalidInput(
                "Broadcast messaging ('*') is not supported".into(),
            ));
        }

        // Early guard: reject targeting parent/main session
        if to == ctx.session_id || to == "main" {
            return Err(SendMessageError::InvalidInput(
                "Cannot send messages to the parent/main session".into(),
            ));
        }

        // --- Extract and validate `message` ---
        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(SendMessageError::MissingField("message"))?
            .to_string();

        // --- Extract `summary` (schema-optional) ---
        let summary = input
            .get("summary")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        // Validation-required: summary must be present for string messages
        // (source: SendMessageTool.ts lines 667-674)
        if summary.is_none() {
            return Err(SendMessageError::InvalidInput(
                "summary is required when message is a string".into(),
            ));
        }

        Ok(SendMessageRequest {
            to,
            message,
            summary,
        })
    }
}

#[async_trait::async_trait]
impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
    }

    fn description(&self) -> &str {
        "Send a message to a running or stopped background agent. The message \
         is delivered at the agent's next tool round boundary. If the agent is \
         stopped, it is automatically resumed with your message."
    }

    fn input_schema(&self) -> serde_json::Value {
        // summary is schema-OPTIONAL (not in "required") but validated as required
        // for string messages at runtime.
        json!({
            "type": "object",
            "required": ["to", "message"],
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient: agent name or agent ID"
                },
                "message": {
                    "type": "string",
                    "description": "The message to send"
                },
                "summary": {
                    "type": "string",
                    "description": "A 5-10 word summary for UI preview (required for string messages)"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        match self.validate_and_build(&input, ctx) {
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
            session_id: "test-session-abc123".into(),
            mode: crate::tool::AgentMode::Normal,
            extra_dirs: vec![],
        }
    }

    // --- Schema tests ---

    #[test]
    fn tool_name_is_send_message() {
        let tool = SendMessageTool;
        assert_eq!(tool.name(), "SendMessage");
    }

    #[test]
    fn schema_requires_to_and_message_but_not_summary() {
        let tool = SendMessageTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");

        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("to")), "schema must require 'to'");
        assert!(
            required.contains(&json!("message")),
            "schema must require 'message'"
        );
        // summary is schema-OPTIONAL — must NOT be in required
        assert!(
            !required.contains(&json!("summary")),
            "summary must NOT be in required (schema-optional)"
        );
    }

    #[test]
    fn schema_has_summary_property() {
        let tool = SendMessageTool;
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("summary"), "schema must define summary property");
        assert_eq!(props["summary"]["type"], "string");
    }

    #[test]
    fn permission_level_is_risky() {
        let tool = SendMessageTool;
        assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Risky);
    }

    // --- Valid input tests ---

    #[tokio::test]
    async fn valid_input_returns_send_message_request() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-42",
            "message": "Please review the parser module",
            "summary": "Review parser module"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);

        let request: SendMessageRequest =
            serde_json::from_str(&result.content).expect("should deserialize");
        assert_eq!(request.to, "agent-42");
        assert_eq!(request.message, "Please review the parser module");
        assert_eq!(request.summary.as_deref(), Some("Review parser module"));
    }

    #[tokio::test]
    async fn valid_request_round_trips_through_json() {
        let request = SendMessageRequest {
            to: "test-agent".into(),
            message: "test message".into(),
            summary: Some("test summary".into()),
        };

        let json_str = serde_json::to_string(&request).expect("serialize");
        let deserialized: SendMessageRequest =
            serde_json::from_str(&json_str).expect("deserialize");
        assert_eq!(request, deserialized);
    }

    #[tokio::test]
    async fn request_without_summary_serializes_without_field() {
        let request = SendMessageRequest {
            to: "test-agent".into(),
            message: "test message".into(),
            summary: None,
        };

        let json_str = serde_json::to_string(&request).expect("serialize");
        assert!(
            !json_str.contains("summary"),
            "None summary should be skipped in serialization"
        );
    }

    // --- Missing/empty field tests ---

    #[tokio::test]
    async fn missing_to_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "message": "hello",
            "summary": "greeting"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("to"),
            "error should mention 'to': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn empty_to_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "   ",
            "message": "hello",
            "summary": "greeting"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("to"),
            "error should mention 'to': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn missing_message_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "summary": "greeting"
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
    async fn empty_message_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message": "",
            "summary": "greeting"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("message"),
            "error should mention 'message': {}",
            result.content
        );
    }

    // --- Summary validation (schema-optional, validation-required) ---

    #[tokio::test]
    async fn missing_summary_returns_error_for_string_message() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message": "Please review this code"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error, "missing summary should be an error");
        assert!(
            result.content.contains("summary"),
            "error should mention summary: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn empty_summary_returns_error() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message": "Please review this code",
            "summary": "   "
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error, "whitespace-only summary should be an error");
        assert!(
            result.content.contains("summary"),
            "error should mention summary: {}",
            result.content
        );
    }

    // --- Early guards ---

    #[tokio::test]
    async fn broadcast_target_star_rejected() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "*",
            "message": "hello everyone",
            "summary": "broadcast greeting"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error, "broadcast should be rejected");
        assert!(
            result.content.contains("Broadcast") || result.content.contains("broadcast"),
            "error should mention broadcast: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn parent_session_id_rejected() {
        let tool = SendMessageTool;
        let ctx = make_ctx();
        let input = json!({
            "to": ctx.session_id,
            "message": "hello parent",
            "summary": "greeting parent"
        });

        let result = tool.execute(input, &ctx).await;
        assert!(result.is_error, "parent session targeting should be rejected");
        assert!(
            result.content.contains("parent") || result.content.contains("main"),
            "error should mention parent/main: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn main_session_keyword_rejected() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "main",
            "message": "hello main",
            "summary": "greeting main"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error, "'main' targeting should be rejected");
        assert!(
            result.content.contains("parent") || result.content.contains("main"),
            "error should mention parent/main: {}",
            result.content
        );
    }

    // --- Agent ID validation ---

    #[test]
    fn is_valid_agent_id_accepts_uuid() {
        assert!(is_valid_agent_id("550e8400-e29b-41d4-a716-446655440000"));
    }

    #[test]
    fn is_valid_agent_id_accepts_structured_id() {
        assert!(is_valid_agent_id("agent-550e8400-e29b-41d4-a716-446655440000"));
    }

    #[test]
    fn is_valid_agent_id_accepts_simple_name() {
        // Names like "explore" or "code-reviewer" pass format check
        // (they're resolved via name registry first in the agent loop)
        assert!(is_valid_agent_id("explore"));
        assert!(is_valid_agent_id("code-reviewer"));
    }

    #[test]
    fn is_valid_agent_id_rejects_empty() {
        assert!(!is_valid_agent_id(""));
    }

    #[test]
    fn is_valid_agent_id_rejects_spaces() {
        assert!(!is_valid_agent_id("agent with spaces"));
    }

    #[test]
    fn is_valid_agent_id_rejects_too_long() {
        let long = "a".repeat(129);
        assert!(!is_valid_agent_id(&long));
    }

    #[test]
    fn is_valid_agent_id_accepts_max_length() {
        let max = "a".repeat(128);
        assert!(is_valid_agent_id(&max));
    }
}
