//! VerbosityToggle tool — switches the TUI between brief and verbose display modes.
//!
//! The agent calls this tool with `action = "toggle" | "set" | "status"`.
//! State is held in a caller-supplied `Arc<Mutex<VerbosityState>>` so both the
//! tool executor and the TUI render loop can read/write the same flag.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// VerbosityState
// ---------------------------------------------------------------------------

/// Shared verbosity state.  `verbose = true` is the default (show everything).
/// `verbose = false` is brief mode (compact rendering).
#[derive(Debug, Clone)]
pub struct VerbosityState {
    verbose: bool,
}

impl VerbosityState {
    /// Create a new state with an explicit initial value.
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }

    /// Returns `true` when in verbose mode.
    pub fn is_verbose(&self) -> bool {
        self.verbose
    }

    /// Flip the current mode.
    pub fn toggle(&mut self) {
        self.verbose = !self.verbose;
    }

    /// Explicitly set verbose (`true`) or brief (`false`).
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// Return `"verbose"` or `"brief"`.
    pub fn mode_str(&self) -> &'static str {
        if self.verbose { "verbose" } else { "brief" }
    }
}

// ---------------------------------------------------------------------------
// VerbosityToggleTool
// ---------------------------------------------------------------------------

/// Tool name exposed to the LLM.
pub const TOOL_NAME: &str = "VerbosityToggle";

/// Tool that the model (or user) calls to control brief/verbose display mode.
pub struct VerbosityToggleTool {
    state: Arc<Mutex<VerbosityState>>,
}

impl VerbosityToggleTool {
    /// Create a new tool backed by the given shared state.
    pub fn new(state: Arc<Mutex<VerbosityState>>) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait]
impl Tool for VerbosityToggleTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Toggle or set the display verbosity mode. \
         In brief mode, tool calls show the name and first line only; \
         tool results show length and first line; thinking shows a placeholder. \
         In verbose mode (default), everything is shown in full."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["toggle", "set", "status"],
                    "description": "toggle: flip the mode; set: use 'mode' to specify; status: return current mode"
                },
                "mode": {
                    "type": "string",
                    "enum": ["brief", "verbose"],
                    "description": "Required when action is 'set'."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::error("VerbosityToggle: 'action' field is required");
            }
        };

        match action {
            "toggle" => {
                let new_mode = {
                    let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                    state.toggle();
                    state.mode_str()
                };
                ToolResult::success(format!("Verbosity mode is now: {new_mode}"))
            }
            "set" => {
                let mode = match input.get("mode").and_then(|v| v.as_str()) {
                    Some(m) => m,
                    None => {
                        return ToolResult::error(
                            "VerbosityToggle: 'mode' field is required for 'set' action",
                        );
                    }
                };
                match mode {
                    "brief" => {
                        self.state
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .set_verbose(false);
                        ToolResult::success("Verbosity mode set to: brief")
                    }
                    "verbose" => {
                        self.state
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .set_verbose(true);
                        ToolResult::success("Verbosity mode set to: verbose")
                    }
                    other => ToolResult::error(format!(
                        "VerbosityToggle: unknown mode '{other}' — use 'brief' or 'verbose'"
                    )),
                }
            }
            "status" => {
                let mode = self
                    .state
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .mode_str();
                ToolResult::success(format!("Current verbosity mode: {mode}"))
            }
            other => ToolResult::error(format!(
                "VerbosityToggle: unknown action '{other}' — use 'toggle', 'set', or 'status'"
            )),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
