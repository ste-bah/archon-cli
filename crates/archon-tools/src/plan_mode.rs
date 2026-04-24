use serde_json::json;

use crate::tool::{AgentMode, PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool to enter plan mode. In plan mode, only read-only tools are allowed.
pub struct EnterPlanModeTool;

#[async_trait::async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> &str {
        "Enter plan mode. Only read-only tools (Read, Glob, Grep, AskUserQuestion) are allowed."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // The agent loop will intercept this and set AgentMode::Plan.
        // The tool itself just signals the intent.
        ToolResult::success("Plan mode entered. Only read-only tools are available.")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

/// Tool to exit plan mode. Re-enables all tools.
pub struct ExitPlanModeTool;

#[async_trait::async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> &str {
        "Exit plan mode. Re-enables all tools."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        if ctx.mode != AgentMode::Plan {
            return ToolResult::error("Not in plan mode. Use EnterPlanMode first.");
        }
        // The agent loop will intercept this and set AgentMode::Normal.
        ToolResult::success("Plan mode exited. All tools are now available.")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

/// Check if a tool is allowed in the current agent mode.
pub fn is_tool_allowed_in_mode(tool_name: &str, mode: AgentMode) -> bool {
    match mode {
        AgentMode::Normal => true,
        AgentMode::Plan => {
            matches!(
                tool_name,
                "Read"
                    | "Glob"
                    | "Grep"
                    | "AskUserQuestion"
                    | "EnterPlanMode"
                    | "ExitPlanMode"
                    | "TaskCreate"
                    | "TaskUpdate"
                    | "TaskGet"
                    | "TaskList"
                    | "Agent"
            )
        }
    }
}

// ---------------------------------------------------------------------------
// TASK-P0-B.3 (#174): D3 bridge from user-facing PermissionMode to the
// dispatch-layer AgentMode. Only PermissionMode::Plan yields
// AgentMode::Plan; every other mode (Default, AcceptEdits, Auto,
// DontAsk, Bubble, BypassPermissions) yields AgentMode::Normal because
// Plan is the only mode that restricts tool dispatch.
//
// archon-tools already depends on archon-permissions (see its
// Cargo.toml), so this impl lives here without introducing a new edge.
// Keeping the bridge beside `is_tool_allowed_in_mode` makes the
// single-mode-restriction invariant easy to audit: any future
// PermissionMode variant added to archon-permissions that needs tool
// restriction must extend BOTH this match AND the match in
// `is_tool_allowed_in_mode` above.
// ---------------------------------------------------------------------------

impl From<&archon_permissions::mode::PermissionMode> for AgentMode {
    /// Map a user-facing [`archon_permissions::mode::PermissionMode`] to the
    /// dispatch-layer [`AgentMode`]. Only `PermissionMode::Plan` yields
    /// `AgentMode::Plan`; every other mode (Default, AcceptEdits, Auto,
    /// DontAsk, Bubble, BypassPermissions) yields `AgentMode::Normal`.
    /// Plan Mode is the only mode that restricts tool dispatch.
    fn from(pm: &archon_permissions::mode::PermissionMode) -> Self {
        match pm {
            archon_permissions::mode::PermissionMode::Plan => AgentMode::Plan,
            _ => AgentMode::Normal,
        }
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::*;
    use archon_permissions::mode::PermissionMode;

    #[test]
    fn plan_bridges_to_plan() {
        assert_eq!(AgentMode::from(&PermissionMode::Plan), AgentMode::Plan);
    }

    #[test]
    fn default_bridges_to_normal() {
        assert_eq!(AgentMode::from(&PermissionMode::Default), AgentMode::Normal);
    }

    #[test]
    fn accept_edits_bridges_to_normal() {
        assert_eq!(
            AgentMode::from(&PermissionMode::AcceptEdits),
            AgentMode::Normal
        );
    }

    #[test]
    fn bypass_bridges_to_normal() {
        assert_eq!(
            AgentMode::from(&PermissionMode::BypassPermissions),
            AgentMode::Normal
        );
    }
}
