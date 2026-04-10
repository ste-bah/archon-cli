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
    /// Structured message type. Defaults to "text" if omitted.
    #[serde(default = "default_message_type")]
    pub message_type: String,
    /// Correlation id for structured request/response pairs (TASK-T2 G2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Approval decision for shutdown_response / plan_approval_response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approve: Option<bool>,
    /// Human-readable reason (shutdown_response).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Human-readable feedback (plan_approval_response).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<String>,
}

fn default_message_type() -> String {
    "text".into()
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

/// Known structured message types that carry an XML envelope instead of text.
const STRUCTURED_TYPES: &[&str] = &["shutdown_request", "shutdown_response", "plan_approval_response"];

/// Message types that require `request_id` + `approve` fields (TASK-T2 G2).
const RESPONSE_TYPES: &[&str] = &["shutdown_response", "plan_approval_response"];

/// HTML-escape inner text so embedded user content cannot break XML parsing.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Build the XML envelope string for structured message types (TASK-T2 G2).
///
/// Format:
/// ```text
/// <archon_structured_message type="shutdown_response" request_id="uuid" approve="true">
/// <reason>optional reason text</reason>
/// </archon_structured_message>
/// ```
///
/// - `reason` is used for `shutdown_response`.
/// - `feedback` is used for `plan_approval_response`.
/// - Optional inner elements are omitted when their source field is `None`.
/// - Inner text is HTML-escaped to prevent malformed XML.
///
/// Text messages (`message_type == "text"`) should NOT use this function.
pub fn build_structured_envelope(req: &SendMessageRequest) -> String {
    let request_id = req.request_id.as_deref().unwrap_or("");
    let approve = req
        .approve
        .map(|b| if b { "true" } else { "false" })
        .unwrap_or("");

    let mut out = format!(
        "<archon_structured_message type=\"{}\" request_id=\"{}\" approve=\"{}\">\n",
        req.message_type, request_id, approve
    );

    if let Some(reason) = req.reason.as_deref() {
        out.push_str(&format!("<reason>{}</reason>\n", xml_escape(reason)));
    }
    if let Some(feedback) = req.feedback.as_deref() {
        out.push_str(&format!("<feedback>{}</feedback>\n", xml_escape(feedback)));
    }

    out.push_str("</archon_structured_message>");
    out
}

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

        // --- Extract `message_type` FIRST — subsequent validation depends on it ---
        let message_type = input
            .get("message_type")
            .and_then(|v| v.as_str())
            .unwrap_or("text")
            .to_string();

        // Unknown message_type is an error (accept "text" or any known structured type)
        let is_text = message_type == "text";
        let is_structured = STRUCTURED_TYPES.contains(&message_type.as_str());
        if !is_text && !is_structured {
            return Err(SendMessageError::InvalidInput(format!(
                "Unknown message_type: '{}' (expected one of: text, shutdown_request, shutdown_response, plan_approval_response)",
                message_type
            )));
        }

        // --- Extract `message` ---
        // For text messages it is required-nonempty. For structured types it is optional
        // (the envelope carries the semantic payload).
        let message = if is_text {
            input
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(SendMessageError::MissingField("message"))?
                .to_string()
        } else {
            input
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        // --- Extract `summary` (schema-optional) ---
        let summary = input
            .get("summary")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        // Validation-required: summary must be present for string (text) messages
        // (source: SendMessageTool.ts lines 667-674). Structured types don't need it.
        if is_text && summary.is_none() {
            return Err(SendMessageError::InvalidInput(
                "summary is required when message is a string".into(),
            ));
        }

        // --- Extract structured-response fields ---
        let request_id = input
            .get("request_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let approve = input.get("approve").and_then(|v| v.as_bool());

        let reason = input
            .get("reason")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let feedback = input
            .get("feedback")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        // --- Response-type validation: request_id + approve both required ---
        if RESPONSE_TYPES.contains(&message_type.as_str()) {
            if request_id.is_none() {
                return Err(SendMessageError::InvalidInput(format!(
                    "request_id is required for {}",
                    message_type
                )));
            }
            if approve.is_none() {
                return Err(SendMessageError::InvalidInput(format!(
                    "approve is required for {}",
                    message_type
                )));
            }
        }

        Ok(SendMessageRequest {
            to,
            message,
            summary,
            message_type,
            request_id,
            approve,
            reason,
            feedback,
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
        // for text messages at runtime. `message` is only required for text messages.
        json!({
            "type": "object",
            "required": ["to"],
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient: agent name or agent ID"
                },
                "message": {
                    "type": "string",
                    "description": "The message to send (required for text messages)"
                },
                "summary": {
                    "type": "string",
                    "description": "A 5-10 word summary for UI preview (required for text messages)"
                },
                "message_type": {
                    "type": "string",
                    "enum": [
                        "text",
                        "shutdown_request",
                        "shutdown_response",
                        "plan_approval_response"
                    ],
                    "description": "Message type. 'text' for plain messages, 'shutdown_request' to request graceful stop, 'shutdown_response' to reply to a shutdown request, 'plan_approval_response' to reply to a plan approval request. Defaults to 'text'."
                },
                "request_id": {
                    "type": "string",
                    "description": "Correlation id of the original request (required for shutdown_response and plan_approval_response)"
                },
                "approve": {
                    "type": "boolean",
                    "description": "Approval decision (required for shutdown_response and plan_approval_response)"
                },
                "reason": {
                    "type": "string",
                    "description": "Optional human-readable reason, used with shutdown_response"
                },
                "feedback": {
                    "type": "string",
                    "description": "Optional human-readable feedback, used with plan_approval_response"
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
    fn schema_requires_to_but_not_summary_or_message() {
        let tool = SendMessageTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");

        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("to")), "schema must require 'to'");
        // After TASK-T2 (G2) `message` is only required at runtime for text messages
        // (structured message types carry their payload via request_id/approve/etc).
        assert!(
            !required.contains(&json!("message")),
            "message must NOT be in schema-required (runtime-required for text only)"
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
        assert!(
            props.contains_key("summary"),
            "schema must define summary property"
        );
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
            message_type: default_message_type(),
            request_id: None,
            approve: None,
            reason: None,
            feedback: None,
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
            message_type: default_message_type(),
            request_id: None,
            approve: None,
            reason: None,
            feedback: None,
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
        assert!(
            result.is_error,
            "whitespace-only summary should be an error"
        );
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
        assert!(
            result.is_error,
            "parent session targeting should be rejected"
        );
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
        assert!(is_valid_agent_id(
            "agent-550e8400-e29b-41d4-a716-446655440000"
        ));
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

    // -----------------------------------------------------------------------
    // TASK-T2 (G2): Structured message types
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn shutdown_request_without_summary_accepted() {
        // shutdown_request is a structured type — summary should not be required
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message": "please stop",
            "message_type": "shutdown_request"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(
            !result.is_error,
            "shutdown_request without summary should be accepted: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn shutdown_response_without_message_accepted() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message_type": "shutdown_response",
            "request_id": "req-abc",
            "approve": true
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(
            !result.is_error,
            "shutdown_response without message should be accepted: {}",
            result.content
        );

        let request: SendMessageRequest =
            serde_json::from_str(&result.content).expect("should deserialize");
        assert_eq!(request.message_type, "shutdown_response");
        assert_eq!(request.request_id.as_deref(), Some("req-abc"));
        assert_eq!(request.approve, Some(true));
    }

    #[tokio::test]
    async fn shutdown_response_without_request_id_rejected() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message_type": "shutdown_response",
            "approve": true
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("request_id"),
            "error should mention request_id: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn shutdown_response_without_approve_rejected() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message_type": "shutdown_response",
            "request_id": "req-abc"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("approve"),
            "error should mention approve: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn plan_approval_response_without_approve_rejected() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message_type": "plan_approval_response",
            "request_id": "req-abc"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("approve"),
            "error should mention approve: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn plan_approval_response_with_all_required_fields_accepted() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message_type": "plan_approval_response",
            "request_id": "req-abc",
            "approve": false,
            "feedback": "please split step 2 into two steps"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(
            !result.is_error,
            "plan_approval_response with all required fields should be accepted: {}",
            result.content
        );

        let request: SendMessageRequest =
            serde_json::from_str(&result.content).expect("should deserialize");
        assert_eq!(request.message_type, "plan_approval_response");
        assert_eq!(request.request_id.as_deref(), Some("req-abc"));
        assert_eq!(request.approve, Some(false));
        assert_eq!(
            request.feedback.as_deref(),
            Some("please split step 2 into two steps")
        );
    }

    #[tokio::test]
    async fn unknown_message_type_rejected() {
        let tool = SendMessageTool;
        let input = json!({
            "to": "agent-1",
            "message": "hello",
            "summary": "greeting",
            "message_type": "bogus"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("message_type") || result.content.contains("bogus"),
            "error should mention unknown message_type: {}",
            result.content
        );
    }

    // --- build_structured_envelope tests ---

    #[test]
    fn build_structured_envelope_shutdown_response_with_reason() {
        let req = SendMessageRequest {
            to: "agent-1".into(),
            message: String::new(),
            summary: None,
            message_type: "shutdown_response".into(),
            request_id: Some("req-1".into()),
            approve: Some(true),
            reason: Some("timeout".into()),
            feedback: None,
        };

        let envelope = build_structured_envelope(&req);
        let expected = "<archon_structured_message type=\"shutdown_response\" request_id=\"req-1\" approve=\"true\">\n<reason>timeout</reason>\n</archon_structured_message>";
        assert_eq!(envelope, expected);
    }

    #[test]
    fn build_structured_envelope_plan_approval_with_feedback() {
        let req = SendMessageRequest {
            to: "agent-1".into(),
            message: String::new(),
            summary: None,
            message_type: "plan_approval_response".into(),
            request_id: Some("req-2".into()),
            approve: Some(false),
            reason: None,
            feedback: Some("needs rework".into()),
        };

        let envelope = build_structured_envelope(&req);
        let expected = "<archon_structured_message type=\"plan_approval_response\" request_id=\"req-2\" approve=\"false\">\n<feedback>needs rework</feedback>\n</archon_structured_message>";
        assert_eq!(envelope, expected);
    }

    #[test]
    fn build_structured_envelope_without_optional_inner() {
        let req = SendMessageRequest {
            to: "agent-1".into(),
            message: String::new(),
            summary: None,
            message_type: "shutdown_response".into(),
            request_id: Some("req-3".into()),
            approve: Some(true),
            reason: None,
            feedback: None,
        };

        let envelope = build_structured_envelope(&req);
        let expected = "<archon_structured_message type=\"shutdown_response\" request_id=\"req-3\" approve=\"true\">\n</archon_structured_message>";
        assert_eq!(envelope, expected);
    }

    #[test]
    fn build_structured_envelope_escapes_special_chars() {
        let req = SendMessageRequest {
            to: "agent-1".into(),
            message: String::new(),
            summary: None,
            message_type: "shutdown_response".into(),
            request_id: Some("req-4".into()),
            approve: Some(true),
            reason: Some("<bad> & \"quote\"".into()),
            feedback: None,
        };

        let envelope = build_structured_envelope(&req);
        assert!(
            envelope.contains("&lt;bad&gt;"),
            "< and > should be escaped: {}",
            envelope
        );
        assert!(
            envelope.contains("&amp;"),
            "& should be escaped: {}",
            envelope
        );
        assert!(
            envelope.contains("&quot;quote&quot;"),
            "\" should be escaped: {}",
            envelope
        );
        // Ensure none of the raw special chars leak inside the <reason> body
        assert!(!envelope.contains("<bad>"));
        assert!(!envelope.contains("\"quote\""));
    }
}
