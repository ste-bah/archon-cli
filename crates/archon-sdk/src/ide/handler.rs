//! JSON-RPC request dispatcher for the IDE protocol (TASK-CLI-411).
//!
//! [`IdeProtocolHandler`] receives raw JSON-RPC request strings, dispatches
//! to the appropriate method handler, and returns a JSON-RPC response string.
//!
//! Every method that needs the agent runs against a live [`IdeAgentRuntime`]
//! (issue #26). Without one the handler answers protocol-shape requests —
//! `archon/initialize`, `archon/config` for process-level keys — and refuses
//! the rest with an explicit error rather than a plausible-looking success.
//! That refusal is the point: the mode with no agent used to answer
//! `archon/prompt` with `{"queued": true}`, so `archon serve` looked like it
//! had accepted a prompt it was never going to run.

use std::collections::HashMap;

use tokio::sync::mpsc;
use uuid::Uuid;

use archon_core::agent::{Agent, TimestampedEvent};

use crate::ide::config;
use crate::ide::protocol::{
    IdeCancelParams, IdeCapabilities, IdeConfigParams, IdeInitializeParams, IdeInitializeResult,
    IdePermissionResponseParams, IdePromptParams, IdeSession, IdeStatusParams, IdeToolResultParams,
    JRpcErrorCode, JRpcNotification, error_response, parse_request, success_response,
};
use crate::ide::runtime::IdeAgentRuntime;

pub use crate::ide::events::event_to_notification;

/// Message returned by every method that cannot work without an agent.
const NO_AGENT: &str = "no agent is attached to this handler, so nothing can run; start the session with `archon ide-stdio`";

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
    /// attached. Prompts are refused.
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
    /// built with. Returns the handler, the notification receiver the
    /// transport drains, and the shared agent handle.
    ///
    /// The agent is taken by value because [`IdeAgentRuntime::new`] installs
    /// the permission channel on it before anything else can hold it — see
    /// there for why that ordering is what makes tools safe to enable.
    pub fn with_agent(
        server_version: impl Into<String>,
        agent: Agent,
        agent_events: mpsc::Receiver<TimestampedEvent>,
    ) -> (
        Self,
        mpsc::Receiver<JRpcNotification>,
        std::sync::Arc<tokio::sync::Mutex<Agent>>,
    ) {
        let (runtime, notifications, agent) = IdeAgentRuntime::new(agent, agent_events);
        (
            Self {
                sessions: HashMap::new(),
                server_version: server_version.into(),
                runtime: Some(runtime),
            },
            notifications,
            agent,
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
            "archon/permissionResponse" => self.handle_permission_response(id, params),
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
            capabilities: init_params.capabilities.clone(),
        };
        self.sessions.insert(session_id.clone(), session);
        // The event pump is already running and needs an id to stamp on
        // outbound notifications; this is the moment one exists. The client's
        // `toolExecution` capability lands here too, because it decides
        // whether a permission prompt has anyone to answer it.
        if let Some(runtime) = &self.runtime {
            runtime.set_session_id(&session_id);
            runtime.set_client_can_approve_tools(init_params.capabilities.tool_execution);
        }

        let result = IdeInitializeResult {
            session_id,
            server_version: self.server_version.clone(),
            capabilities: IdeCapabilities {
                // The server runs tools itself and asks before the dangerous
                // ones; the other three are still client-side surfaces it has
                // no part in.
                tool_execution: self.runtime.is_some(),
                ..IdeCapabilities::default()
            },
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

        if let Some(error) = self.reject_unknown_session(id, &prompt_params.session_id) {
            return error;
        }

        let Some(runtime) = self.runtime.as_mut() else {
            return error_response(id, JRpcErrorCode::INVALID_REQUEST, NO_AGENT);
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

        if let Some(error) = self.reject_unknown_session(id, &cancel_params.session_id) {
            return error;
        }

        // `false` means "there was nothing to stop", not "the request
        // failed" — cancelling an idle session is a no-op, not an error.
        let cancelled = match self.runtime.as_mut() {
            Some(runtime) => runtime.cancel_turn(),
            None => false,
        };
        success_response(id, serde_json::json!({"cancelled": cancelled}))
    }

    fn handle_permission_response(&mut self, id: u64, params: serde_json::Value) -> String {
        let response: IdePermissionResponseParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/permissionResponse params: {e}"),
                );
            }
        };

        if let Some(error) = self.reject_unknown_session(id, &response.session_id) {
            return error;
        }

        let Some(runtime) = self.runtime.as_ref() else {
            return error_response(id, JRpcErrorCode::INVALID_REQUEST, NO_AGENT);
        };

        // An answer nobody is waiting for is an error, not a quiet success:
        // the user pressed a button and needs to know it did nothing.
        match runtime.respond_to_permission(&response.request_id, response.approved) {
            Ok(()) => success_response(id, serde_json::json!({"delivered": true})),
            Err(reason) => error_response(id, JRpcErrorCode::INVALID_REQUEST, &reason),
        }
    }

    /// `archon/toolResult` — explicitly unsupported.
    ///
    /// The method exists in the protocol for a client that executes tools on
    /// the agent's behalf. Archon does not work that way: `Agent` dispatches
    /// every tool in-process through its own registry and never waits on the
    /// IDE for a result, so there is nothing for a result to be delivered to.
    /// Answering `{"ok": true}` — which this did — told the client its result
    /// had been consumed when it had been dropped on the floor.
    fn handle_tool_result(&mut self, id: u64, params: serde_json::Value) -> String {
        let tool_params: IdeToolResultParams = match serde_json::from_value(params) {
            Ok(p) => p,
            Err(e) => {
                return error_response(
                    id,
                    JRpcErrorCode::INVALID_PARAMS,
                    &format!("invalid archon/toolResult params: {e}"),
                );
            }
        };

        error_response(
            id,
            JRpcErrorCode::INVALID_REQUEST,
            &format!(
                "archon/toolResult is not supported: Archon executes tools in-process, so no \
                 result is expected from the client (dropped result for toolUseId {})",
                tool_params.tool_use_id
            ),
        )
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

        if let Some(error) = self.reject_unknown_session(id, &status_params.session_id) {
            return error;
        }

        let Some(runtime) = self.runtime.as_ref() else {
            return error_response(id, JRpcErrorCode::INVALID_REQUEST, NO_AGENT);
        };

        match serde_json::to_value(runtime.status()) {
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

        let Some(key) = config_params.key.as_deref() else {
            return error_response(
                id,
                JRpcErrorCode::INVALID_PARAMS,
                &format!(
                    "archon/config requires a key; known keys are {}",
                    config::KNOWN_KEYS.join(", ")
                ),
            );
        };

        let runtime = self.runtime.as_ref();
        let outcome = match config_params.value.as_ref() {
            Some(value) => config::write(runtime, key, value)
                .map(|()| serde_json::json!({"ok": true, "key": key})),
            None => config::read(runtime, key).map(|value| serde_json::json!({"value": value})),
        };

        match outcome {
            Ok(result) => success_response(id, result),
            Err(reason) => error_response(id, JRpcErrorCode::INVALID_PARAMS, &reason),
        }
    }

    /// Common guard: every session-scoped method must refuse an id it never
    /// issued rather than acting on the one session it happens to have.
    fn reject_unknown_session(&self, id: u64, session_id: &str) -> Option<String> {
        if self.sessions.contains_key(session_id) {
            return None;
        }
        Some(error_response(
            id,
            JRpcErrorCode::INVALID_PARAMS,
            &format!("unknown sessionId: {session_id}"),
        ))
    }
}
