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
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub max_turns: u32,
    pub timeout_secs: u64,
    /// When set, loads a custom agent definition for this subagent.
    #[serde(default)]
    pub subagent_type: Option<String>,
    /// Per-call background override. When true, subagent runs as a background task.
    #[serde(default)]
    pub run_in_background: bool,
    /// Working directory override for the subagent.
    #[serde(default)]
    pub cwd: Option<String>,
    /// When set to "worktree", the subagent runs in an isolated git worktree.
    #[serde(default)]
    pub isolation: Option<String>,
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

pub struct AgentTool {
    /// Dynamic description including available agents. Built at registration time.
    description: String,
}

impl AgentTool {
    /// Create an AgentTool with default description (no agent listing).
    pub fn new() -> Self {
        Self {
            description: "Spawn a subagent to handle a complex task autonomously. Returns a SubagentRequest \
                for the agent loop to execute. The subagent runs with its own conversation and \
                tool set.".into(),
        }
    }

    /// Create an AgentTool with an injected agent listing.
    /// The listing is appended to the description so the LLM knows valid subagent_type values.
    pub fn with_agent_listing(agents: &[(String, String)]) -> Self {
        let mut desc = "Spawn a subagent to handle a complex task autonomously. Returns a SubagentRequest \
            for the agent loop to execute. The subagent runs with its own conversation and \
            tool set.".to_string();

        if !agents.is_empty() {
            desc.push_str("\n\nAvailable agents: ");
            let entries: Vec<String> = agents
                .iter()
                .map(|(name, summary)| {
                    if summary.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} ({summary})")
                    }
                })
                .collect();
            desc.push_str(&entries.join(", "));
        }

        Self { description: desc }
    }
}

impl Default for AgentTool {
    fn default() -> Self {
        Self::new()
    }
}

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

        let subagent_type = input
            .get("subagent_type")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let cwd = input
            .get("cwd")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let isolation = input
            .get("isolation")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        Ok(SubagentRequest {
            prompt,
            model,
            allowed_tools,
            max_turns,
            timeout_secs,
            subagent_type,
            run_in_background,
            cwd,
            isolation,
        })
    }
}

#[async_trait::async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }

    fn description(&self) -> &str {
        &self.description
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
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Optional agent type name. When set, loads the agent's custom prompt and tool filters."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "When true, runs the subagent as a background task."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory override for the subagent."
                },
                "isolation": {
                    "type": "string",
                    "enum": ["worktree"],
                    "description": "Isolation mode. 'worktree' creates a temporary git worktree for the subagent."
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
            extra_dirs: vec![],
        }
    }

    #[tokio::test]
    async fn valid_input_returns_subagent_request() {
        let tool = AgentTool::new();
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
        assert!(!request.run_in_background);
        assert!(request.cwd.is_none());
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let tool = AgentTool::new();
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
        let tool = AgentTool::new();
        let input = json!({ "prompt": "   " });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("prompt"));
    }

    #[tokio::test]
    async fn default_max_turns_applied() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Do something" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
        assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
        assert!(!request.run_in_background);
        assert!(request.cwd.is_none());
    }

    #[tokio::test]
    async fn allowed_tools_parsed_from_array() {
        let tool = AgentTool::new();
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
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Analyze code" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert!(request.allowed_tools.is_empty());
    }

    #[tokio::test]
    async fn invalid_max_turns_returns_error() {
        let tool = AgentTool::new();

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
        let tool = AgentTool::new();
        assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Risky);
    }

    #[tokio::test]
    async fn subagent_type_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "subagent_type": "code-reviewer"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert_eq!(request.subagent_type.as_deref(), Some("code-reviewer"));
    }

    #[tokio::test]
    async fn subagent_type_none_when_absent() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Do something" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert!(request.subagent_type.is_none());
    }

    #[test]
    fn subagent_type_backward_compatible_deserialization() {
        // JSON without subagent_type should deserialize fine (serde default)
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(request.subagent_type.is_none());
    }

    #[test]
    fn subagent_type_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: Some("code-reviewer".into()),
            run_in_background: false,
            cwd: None,
            isolation: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["subagent_type"], "code-reviewer");
    }

    #[test]
    fn schema_includes_subagent_type() {
        let tool = AgentTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("subagent_type"));
        assert_eq!(props["subagent_type"]["type"], "string");
    }

    #[tokio::test]
    async fn run_in_background_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "run_in_background": true
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert!(request.run_in_background);
    }

    #[test]
    fn run_in_background_defaults_to_false() {
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(!request.run_in_background);
    }

    #[test]
    fn run_in_background_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: true,
            cwd: None,
            isolation: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["run_in_background"], true);
    }

    #[tokio::test]
    async fn cwd_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "cwd": "/tmp"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert_eq!(request.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn cwd_defaults_to_none() {
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(request.cwd.is_none());
    }

    #[test]
    fn cwd_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: Some("/tmp".into()),
            isolation: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["cwd"], "/tmp");
    }

    // -----------------------------------------------------------------------
    // Worktree isolation tests (AGT-017)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn isolation_worktree_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "isolation": "worktree"
        });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert_eq!(request.isolation.as_deref(), Some("worktree"));
    }

    #[tokio::test]
    async fn isolation_none_when_absent() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Do something" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error);

        let request: SubagentRequest = serde_json::from_str(&result.content).unwrap();
        assert!(request.isolation.is_none());
    }

    #[test]
    fn isolation_backward_compatible_deserialization() {
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(request.isolation.is_none());
    }

    #[test]
    fn isolation_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: Some("worktree".into()),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["isolation"], "worktree");
    }

    #[test]
    fn schema_includes_isolation() {
        let tool = AgentTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("isolation"));
        assert_eq!(props["isolation"]["type"], "string");
    }

    #[test]
    fn schema_includes_run_in_background_and_cwd() {
        let tool = AgentTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("run_in_background"));
        assert_eq!(props["run_in_background"]["type"], "boolean");
        assert!(props.contains_key("cwd"));
        assert_eq!(props["cwd"]["type"], "string");
    }

    #[test]
    fn tool_metadata() {
        let tool = AgentTool::new();
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
