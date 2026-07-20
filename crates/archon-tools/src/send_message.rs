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
const STRUCTURED_TYPES: &[&str] = &[
    "shutdown_request",
    "shutdown_response",
    "plan_approval_response",
];

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
#[path = "send_message_tests/mod.rs"]
mod tests;
