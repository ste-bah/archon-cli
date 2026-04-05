use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Subagent request — returned as JSON for the caller (agent loop) to handle
// ---------------------------------------------------------------------------

/// A validated request to spawn a subagent.  The `AgentTool` does not actually
/// spawn anything — it validates parameters and produces this struct so the
/// outer agent loop can orchestrate the real subagent lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub allowed_tools: Vec<String>,
    pub max_turns: u32,
    pub timeout_secs: u64,
}

impl SubagentRequest {
    /// Default maximum turns when the caller does not specify one.
    pub const DEFAULT_MAX_TURNS: u32 = 10;

    /// Default timeout in seconds.
    pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentToolError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

// ---------------------------------------------------------------------------
// AgentTool — implements Tool
// ---------------------------------------------------------------------------

pub struct AgentTool;

impl AgentTool {
    fn validate_and_build(
        &self,
        input: &serde_json::Value,
    ) -> Result<SubagentRequest, AgentToolError> {
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(AgentToolError::MissingField("prompt"))?
            .to_string();

        let model = input
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let allowed_tools = match input.get("allowed_tools") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };

        let max_turns = input
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| {
                if n == 0 || n > 100 {
                    Err(AgentToolError::InvalidInput(
                        "max_turns must be between 1 and 100".into(),
                    ))
                } else {
                    Ok(n as u32)
                }
            })
            .transpose()?
            .unwrap_or(SubagentRequest::DEFAULT_MAX_TURNS);

        let timeout_secs = input
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(SubagentRequest::DEFAULT_TIMEOUT_SECS);

        Ok(SubagentRequest {
            prompt,
            model,
            allowed_tools,
            max_turns,
            timeout_secs,
        })
    }
}

#[async_trait::async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }

    fn description(&self) -> &str {
        "Spawn a subagent to handle a complex task autonomously. Returns a SubagentRequest \
         for the agent loop to execute. The subagent runs with its own conversation and \
         tool set."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task prompt for the subagent"
                },
                "model": {
                    "type": "string",
                    "description": "Model to use for the subagent (defaults to parent model)"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of tool names the subagent is allowed to use"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum conversation turns (default 10, max 100)"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        match self.validate_and_build(&input) {
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
            session_id: "test-session".into(),
            mode: crate::tool::AgentMode::Normal,
        }
    }

    #[tokio::test]
    async fn valid_input_returns_subagent_request() {
        let tool = AgentTool;
        let input = json!({
            "prompt": "Summarize the codebase",
            "model": "claude-sonnet-4-6",
            "allowed_tools": ["Read", "Glob"],
            "max_turns": 5
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);

        let request: SubagentRequest =
            serde_json::from_str(&result.content).expect("should deserialize");
        assert_eq!(request.prompt, "Summarize the codebase");
        assert_eq!(request.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(request.allowed_tools, vec!["Read", "Glob"]);
        assert_eq!(request.max_turns, 5);
        assert_eq!(request.timeout_secs, 300);
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let tool = AgentTool;
        let input = json!({ "model": "claude-sonnet-4-6" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("prompt"),
            "error should mention 'prompt': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn empty_prompt_returns_error() {
        let tool = AgentTool;
        let input = json!({ "prompt": "   " });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("prompt"));
    }

    #[tokio::test]
    async fn default_max_turns_applied() {
        let tool = AgentTool;
        let input = json!({ "prompt": "Do something" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
        assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
    }

    #[tokio::test]
    async fn allowed_tools_parsed_from_array() {
        let tool = AgentTool;
        let input = json!({
            "prompt": "Refactor module",
            "allowed_tools": ["Read", "Write", "Edit"]
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert_eq!(request.allowed_tools, vec!["Read", "Write", "Edit"]);
    }

    #[tokio::test]
    async fn no_allowed_tools_gives_empty_vec() {
        let tool = AgentTool;
        let input = json!({ "prompt": "Analyze code" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert!(request.allowed_tools.is_empty());
    }

    #[tokio::test]
    async fn invalid_max_turns_returns_error() {
        let tool = AgentTool;

        // Zero
        let result = tool
            .execute(json!({"prompt": "x", "max_turns": 0}), &make_ctx())
            .await;
        assert!(result.is_error);

        // Over 100
        let result = tool
            .execute(json!({"prompt": "x", "max_turns": 101}), &make_ctx())
            .await;
        assert!(result.is_error);
    }

    #[test]
    fn permission_level_is_risky() {
        let tool = AgentTool;
        assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Risky);
    }

    #[test]
    fn tool_metadata() {
        let tool = AgentTool;
        assert_eq!(tool.name(), "Agent");
        assert!(!tool.description().is_empty());

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("prompt"))
        );
    }
}
