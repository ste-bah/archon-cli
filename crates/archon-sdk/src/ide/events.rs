//! Agent event → IDE notification mapping (issue #26).
//!
//! Split out of `handler.rs` so the dispatcher stays a JSON-RPC router. Two
//! entry points, and the split between them is deliberate:
//! [`event_to_notification`] is pure and covers every event whose notification
//! can be built from the event alone; [`notification_for`] is what the runtime
//! pump calls, because permission events additionally need the
//! [`PermissionBridge`] to mint and retire the correlation id.

use archon_core::agent::AgentEvent;

use crate::ide::permission::PermissionBridge;
use crate::ide::protocol::{
    IdeError, IdePermissionRequest, IdePermissionResolved, IdeTextDelta, IdeThinkingDelta,
    IdeToolCall, IdeToolCallComplete, IdeTurnComplete, JRpcErrorCode, JRpcNotification,
};

/// Longest tool output forwarded to the IDE for display.
///
/// A `Read` of a large file or a chatty build can run to megabytes, and the
/// whole thing would have to cross stdout as one JSON line before any later
/// delta could be written. The IDE renders a preview, so it is truncated here
/// rather than at the far end where the cost has already been paid.
const TOOL_OUTPUT_PREVIEW_LIMIT: usize = 4096;

/// Map an [`AgentEvent`] to an IDE notification, if applicable.
///
/// Returns `None` for events that have no IDE notification equivalent
/// (e.g. `UserPromptReady`, `CompactionTriggered`), and for the three
/// permission events — those carry a correlation id that only the runtime can
/// mint, so they are built by [`notification_for`] instead. Emitting an
/// id-less `archon/permissionRequest` from here would produce a prompt the IDE
/// cannot answer.
pub fn event_to_notification(session_id: &str, event: &AgentEvent) -> Option<JRpcNotification> {
    let (method, params) = match event {
        AgentEvent::TextDelta(text) => (
            "archon/textDelta",
            serde_json::to_value(IdeTextDelta {
                session_id: session_id.to_string(),
                text: text.clone(),
            })
            .ok()?,
        ),
        AgentEvent::ThinkingDelta(text) => (
            "archon/thinkingDelta",
            serde_json::to_value(IdeThinkingDelta {
                session_id: session_id.to_string(),
                thinking: text.clone(),
            })
            .ok()?,
        ),
        AgentEvent::ToolCallStarted { name, id } => (
            "archon/toolCall",
            serde_json::to_value(IdeToolCall {
                session_id: session_id.to_string(),
                tool_use_id: id.clone(),
                name: name.clone(),
                input: serde_json::Value::Null,
            })
            .ok()?,
        ),
        AgentEvent::ToolCallComplete {
            name, id, result, ..
        } => (
            "archon/toolCallComplete",
            serde_json::to_value(IdeToolCallComplete {
                session_id: session_id.to_string(),
                tool_use_id: id.clone(),
                name: name.clone(),
                is_error: result.is_error,
                content: truncate_for_display(&result.content),
            })
            .ok()?,
        ),
        AgentEvent::TurnComplete {
            input_tokens,
            output_tokens,
            ..
        } => (
            "archon/turnComplete",
            serde_json::to_value(IdeTurnComplete {
                session_id: session_id.to_string(),
                input_tokens: *input_tokens,
                output_tokens: *output_tokens,
                cost: 0.0, // Per-turn cost is not on the event; `archon/status` carries the session figure.
            })
            .ok()?,
        ),
        AgentEvent::Error(msg) => (
            "archon/error",
            serde_json::to_value(IdeError {
                session_id: Some(session_id.to_string()),
                message: msg.clone(),
                code: JRpcErrorCode::INTERNAL_ERROR,
            })
            .ok()?,
        ),
        // Events without IDE notification equivalents, plus the permission
        // events that `notification_for` owns.
        _ => return None,
    };

    Some(notification(method, params))
}

/// Build the notification for `event`, minting or retiring a permission
/// correlation id on `bridge` when the event is a permission event.
pub(crate) fn notification_for(
    bridge: &PermissionBridge,
    session_id: &str,
    event: &AgentEvent,
) -> Option<JRpcNotification> {
    match event {
        AgentEvent::PermissionRequired { tool, description } => {
            // Opened before the notification is sent, so an answer that races
            // back in cannot arrive before the bridge will accept it.
            let request_id = bridge.open_request();
            let params = serde_json::to_value(IdePermissionRequest {
                session_id: session_id.to_string(),
                request_id,
                action: tool.clone(),
                description: description.clone(),
            })
            .ok()?;
            if !bridge.client_can_answer() {
                // Still notified, so the user sees what was refused and why —
                // but refused now rather than after the agent's own two-minute
                // timeout reaches the same answer.
                tracing::warn!(
                    tool = %tool,
                    "IDE client advertised no approval UI; refusing the permission request"
                );
                bridge.deny_unanswerable();
            }
            Some(notification("archon/permissionRequest", params))
        }
        AgentEvent::PermissionGranted { tool } => {
            bridge.close_request();
            let params = serde_json::to_value(IdePermissionResolved {
                session_id: session_id.to_string(),
                action: tool.clone(),
                granted: true,
                reason: None,
            })
            .ok()?;
            Some(notification("archon/permissionResolved", params))
        }
        AgentEvent::PermissionDenied { tool, reason } => {
            bridge.close_request();
            let params = serde_json::to_value(IdePermissionResolved {
                session_id: session_id.to_string(),
                action: tool.clone(),
                granted: false,
                reason: reason.clone(),
            })
            .ok()?;
            Some(notification("archon/permissionResolved", params))
        }
        other => event_to_notification(session_id, other),
    }
}

fn notification(method: &str, params: serde_json::Value) -> JRpcNotification {
    JRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
    }
}

fn truncate_for_display(content: &str) -> String {
    if content.len() <= TOOL_OUTPUT_PREVIEW_LIMIT {
        return content.to_string();
    }
    // Slicing on a byte index would panic mid-codepoint; walk back to a
    // boundary rather than trusting the limit to land on one.
    let mut end = TOOL_OUTPUT_PREVIEW_LIMIT;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n… ({} more bytes)",
        &content[..end],
        content.len() - end
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_thinking_has_no_ide_notification() {
        let event = AgentEvent::TransientThinkingDelta("unapproved".into());

        assert!(event_to_notification("session", &event).is_none());
    }

    /// A permission prompt the IDE cannot answer is worse than none: the user
    /// clicks allow, the answer is refused for want of an id, and the agent
    /// sits until it times out. The pure mapper must stay out of it.
    #[test]
    fn the_pure_mapper_refuses_to_build_an_unanswerable_permission_prompt() {
        let event = AgentEvent::PermissionRequired {
            tool: "Bash".into(),
            description: "run a command".into(),
        };

        assert!(event_to_notification("session", &event).is_none());
    }

    #[test]
    fn a_permission_request_carries_the_id_the_ide_must_echo() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let bridge = PermissionBridge::new(tx);
        bridge.set_client_can_answer(true);
        let event = AgentEvent::PermissionRequired {
            tool: "Write".into(),
            description: "write a file".into(),
        };

        let notification = notification_for(&bridge, "session", &event).expect("notification");

        assert_eq!(notification.method, "archon/permissionRequest");
        let request_id = notification.params["requestId"]
            .as_str()
            .expect("requestId");
        assert!(bridge.is_waiting());
        bridge.respond(request_id, true).expect("id round-trips");
    }

    #[tokio::test]
    async fn a_client_with_no_approval_ui_is_told_and_refused_at_once() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let bridge = PermissionBridge::new(tx);
        let event = AgentEvent::PermissionRequired {
            tool: "Bash".into(),
            description: "run a command".into(),
        };

        let notification = notification_for(&bridge, "session", &event).expect("notification");

        assert_eq!(notification.method, "archon/permissionRequest");
        assert_eq!(rx.recv().await, Some(false), "must refuse, not approve");
    }

    #[test]
    fn multibyte_tool_output_is_truncated_on_a_character_boundary() {
        let content = "é".repeat(TOOL_OUTPUT_PREVIEW_LIMIT);

        let truncated = truncate_for_display(&content);

        assert!(truncated.contains("more bytes"));
        assert!(truncated.starts_with('é'));
    }
}
