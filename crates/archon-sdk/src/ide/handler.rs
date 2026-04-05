//! JSON-RPC request dispatcher for the IDE protocol (TASK-CLI-411).
//!
//! [`IdeProtocolHandler`] receives raw JSON-RPC request strings, dispatches
//! to the appropriate method handler, and returns a JSON-RPC response string.
//! Phase 6 will wire the prompt/cancel/toolResult methods to the real agent
//! loop; for now they return the correct protocol-level shapes.

use std::collections::HashMap;

use uuid::Uuid;

use crate::ide::protocol::{
    IdeCancelParams, IdeCapabilities, IdeConfigParams, IdeInitializeParams, IdeInitializeResult,
    IdePromptParams, IdeSession, IdeStatusParams, IdeStatusResult, IdeToolResultParams,
    JRpcErrorCode, error_response, parse_request, success_response,
};

// ── IdeProtocolHandler ────────────────────────────────────────────────────────

/// Stateful JSON-RPC dispatcher for the IDE protocol.
///
/// Holds open IDE sessions (keyed by session ID) and maps incoming JSON-RPC
/// method strings to the correct handler functions.
pub struct IdeProtocolHandler {
    sessions: HashMap<String, IdeSession>,
    server_version: String,
}

impl IdeProtocolHandler {
    /// Create a new handler advertising `server_version`.
    pub fn new(server_version: impl Into<String>) -> Self {
        Self {
            sessions: HashMap::new(),
            server_version: server_version.into(),
        }
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

        // Phase 6: enqueue prompt to agent loop.
        success_response(id, serde_json::json!({"queued": true}))
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

        let cancelled = self.sessions.contains_key(&cancel_params.session_id);
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
