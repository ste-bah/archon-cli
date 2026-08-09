//! JSON-RPC request dispatcher for the IDE protocol (TASK-CLI-411).
//!
//! [`IdeProtocolHandler`] receives raw JSON-RPC request strings, dispatches
//! to the appropriate method handler, and returns a JSON-RPC response string.
//!
//! `archon/prompt` and `archon/cancel` run against a live agent once an
//! [`IdeAgentRuntime`] is attached (issue #26). Without one the handler is a
//! protocol-only echo that still answers with the correct shapes — the mode
//! the synchronous `StdioTransport::run` loop and the protocol shape tests
//! use. `archon/toolResult` remains a stub: tool execution is a later slice.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use archon_core::agent::{Agent, AgentEvent, TimestampedEvent};

use crate::ide::protocol::{
    IdeCancelParams, IdeCapabilities, IdeConfigParams, IdeError, IdeInitializeParams,
    IdeInitializeResult, IdePermissionRequest, IdePromptParams, IdeSession, IdeStatusParams,
    IdeStatusResult, IdeTextDelta, IdeThinkingDelta, IdeToolCall, IdeToolResultParams,
    IdeTurnComplete, JRpcErrorCode, JRpcNotification, error_response, parse_request,
    success_response,
};
use crate::ide::runtime::IdeAgentRuntime;

// ── IdeProtocolHandler ────────────────────────────────────────────────────────

/// Stateful JSON-RPC dispatcher for the IDE protocol.
///
/// Holds open IDE sessions (keyed by session ID) and maps incoming JSON-RPC
/// method strings to the correct handler functions.
pub struct IdeProtocolHandler {
    sessions: HashMap<String, IdeSession>,
    server_version: String,
    /// Agent driving `archon/prompt`. `None` keeps the handler protocol-only.
    runtime: Option<IdeAgentRuntime>,
}

impl IdeProtocolHandler {
    /// Create a new handler advertising `server_version`, with no agent
    /// attached. Prompts are acknowledged but nothing runs.
    pub fn new(server_version: impl Into<String>) -> Self {
        Self {
            sessions: HashMap::new(),
            server_version: server_version.into(),
            runtime: None,
        }
    }

    /// Create a handler that drives `agent` on `archon/prompt`.
    ///
    /// `agent_events` must be the receiver paired with the sender `agent` was
    /// built with. Returns the handler plus the notification receiver the
    /// transport drains; see [`IdeAgentRuntime::new`] for the tool-freedom
    /// precondition on `agent`.
    pub fn with_agent(
        server_version: impl Into<String>,
        agent: Arc<Mutex<Agent>>,
        agent_events: mpsc::Receiver<TimestampedEvent>,
    ) -> (Self, mpsc::Receiver<JRpcNotification>) {
        let (runtime, notifications) = IdeAgentRuntime::new(agent, agent_events);
        (
            Self {
                sessions: HashMap::new(),
                server_version: server_version.into(),
                runtime: Some(runtime),
            },
            notifications,
        )
    }

    /// Handle a raw JSON-RPC request string and return a JSON-RPC response string.
    ///
    /// Returns a JSON-RPC error response for malformed JSON or unknown methods.
    pub fn handle(&mut self, request_json: &str) -> String {
        let (id, method, params) = match parse_request(request_json) {
            Ok(t) => t,
            Err(e) => {
                // Cannot extract an id — use id=0 per JSON-RPC spec for parse errors.
                return error_response(0, JRpcErrorCode::PARSE_ERROR, &e.to_string());
            }
        };

        match method.as_str() {
            "archon/initialize" => self.handle_initialize(id, params),
            "archon/prompt" => self.handle_prompt(id, params),
            "archon/cancel" => self.handle_cancel(id, params),
            "archon/toolResult" => self.handle_tool_result(id, params),
            "archon/status" => self.handle_status(id, params),
            "archon/config" => self.handle_config(id, params),
            other => error_response(
                id,
                JRpcErrorCode::METHOD_NOT_FOUND,
                &format!("method not found: {other}"),
            ),
        }
    }

    // ── Method handlers ───────────────────────────────────────────────────────

    fn handle_initialize(&mut self, id: u64, params: serde_json::Value) -> String {
        let init_params: IdeInitializeParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/initialize params: {e}"),
                );
            }
        };

        let session_id = Uuid::new_v4().to_string();
        let session = IdeSession {
            session_id: session_id.clone(),
            capabilities: IdeCapabilities {
                inline_completion: init_params.capabilities.inline_completion,
                tool_execution: init_params.capabilities.tool_execution,
                diff: init_params.capabilities.diff,
                terminal: init_params.capabilities.terminal,
            },
        };
        self.sessions.insert(session_id.clone(), session);
        // The event pump is already running and needs an id to stamp on
        // outbound notifications; this is the moment one exists.
        if let Some(runtime) = &self.runtime {
            runtime.set_session_id(&session_id);
        }

        let result = IdeInitializeResult {
            session_id,
            server_version: self.server_version.clone(),
            capabilities: IdeCapabilities::default(),
        };

        match serde_json::to_value(&result) {
            Ok(v) => success_response(id, v),
            Err(e) => error_response(id, JRpcErrorCode::INTERNAL_ERROR, &e.to_string()),
        }
    }

    fn handle_prompt(&mut self, id: u64, params: serde_json::Value) -> String {
        let prompt_params: IdePromptParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/prompt params: {e}"),
                );
            }
        };

        if !self.sessions.contains_key(&prompt_params.session_id) {
            return error_response(
                id,
                JRpcErrorCode::INVALID_PARAMS,
                &format!("unknown sessionId: {}", prompt_params.session_id),
            );
        }

        let Some(runtime) = self.runtime.as_mut() else {
            // Protocol-only mode: accept the prompt so the handshake and the
            // transport tests keep working, but nothing runs.
            return success_response(id, serde_json::json!({"queued": true}));
        };

        match runtime.start_turn(&prompt_params) {
            Ok(()) => success_response(id, serde_json::json!({"queued": true})),
            Err(reason) => error_response(id, JRpcErrorCode::INVALID_REQUEST, &reason),
        }
    }

    fn handle_cancel(&mut self, id: u64, params: serde_json::Value) -> String {
        let cancel_params: IdeCancelParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/cancel params: {e}"),
                );
            }
        };

        if !self.sessions.contains_key(&cancel_params.session_id) {
            return error_response(
                id,
                JRpcErrorCode::INVALID_PARAMS,
                &format!("unknown sessionId: {}", cancel_params.session_id),
            );
        }

        // `false` means "there was nothing to stop", not "the request
        // failed" — cancelling an idle session is a no-op, not an error.
        let cancelled = match self.runtime.as_mut() {
            Some(runtime) => runtime.cancel_turn(),
            None => false,
        };
        success_response(id, serde_json::json!({"cancelled": cancelled}))
    }

    fn handle_tool_result(&mut self, id: u64, params: serde_json::Value) -> String {
        let _tool_params: IdeToolResultParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/toolResult params: {e}"),
                );
            }
        };

        // Phase 6: forward result to the waiting agent turn.
        success_response(id, serde_json::json!({"ok": true}))
    }

    fn handle_status(&mut self, id: u64, params: serde_json::Value) -> String {
        let status_params: IdeStatusParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/status params: {e}"),
                );
            }
        };

        if !self.sessions.contains_key(&status_params.session_id) {
            return error_response(
                id,
                JRpcErrorCode::INVALID_PARAMS,
                &format!("unknown sessionId: {}", status_params.session_id),
            );
        }

        // Phase 6: pull real metrics from the agent loop.
        let result = IdeStatusResult {
            model: "claude-sonnet-4-6".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cost: 0.0,
        };

        match serde_json::to_value(&result) {
            Ok(v) => success_response(id, v),
            Err(e) => error_response(id, JRpcErrorCode::INTERNAL_ERROR, &e.to_string()),
        }
    }

    fn handle_config(&mut self, id: u64, params: serde_json::Value) -> String {
        let config_params: IdeConfigParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/config params: {e}"),
                );
            }
        };

        if config_params.value.is_some() {
            success_response(id, serde_json::json!({"ok": true}))
        } else {
            // Phase 6: look up real config values.
            success_response(id, serde_json::json!({"value": null}))
        }
    }
}

/// Map an [`AgentEvent`] to an IDE notification, if applicable.
///
/// Returns `None` for events that have no IDE notification equivalent
/// (e.g. `UserPromptReady`, `CompactionTriggered`, `PermissionGranted`/`Denied`).
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
        AgentEvent::PermissionRequired { tool, description } => (
            "archon/permissionRequest",
            serde_json::to_value(IdePermissionRequest {
                session_id: session_id.to_string(),
                action: tool.clone(),
                description: description.clone(),
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
                cost: 0.0, // Cost calculation not available at this level
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
        // Events without IDE notification equivalents
        _ => return None,
    };

    Some(JRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_thinking_has_no_ide_notification() {
        let event = AgentEvent::TransientThinkingDelta("unapproved".into());

        assert!(event_to_notification("session", &event).is_none());
    }
}
