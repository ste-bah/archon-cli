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
                "Read" | "Glob" | "Grep" | "AskUserQuestion"
                    | "EnterPlanMode" | "ExitPlanMode"
                    | "TaskCreate" | "TaskUpdate" | "TaskGet" | "TaskList"
            )
        }
    }
}
