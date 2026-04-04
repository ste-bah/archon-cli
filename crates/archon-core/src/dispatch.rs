use std::collections::HashMap;

use archon_tools::plan_mode::is_tool_allowed_in_mode;
use archon_tools::tool::{Tool, ToolContext, ToolResult};

/// Registry of available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    /// Get all tool names.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Keep only the tools whose names appear in the whitelist.
    ///
    /// Any tool not in `names` is removed from the registry.
    pub fn filter_whitelist(&mut self, names: &[&str]) {
        self.tools.retain(|k, _| names.contains(&k.as_str()));
    }

    /// Remove tools whose names appear in the blacklist.
    pub fn filter_blacklist(&mut self, names: &[&str]) {
        self.tools.retain(|k, _| !names.contains(&k.as_str()));
    }

    /// Get tool definitions for API request (JSON schemas).
    pub fn tool_definitions(&self) -> Vec<serde_json::Value> {
        self.tools
            .values()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema(),
                })
            })
            .collect()
    }

    /// Dispatch a tool call: check mode, execute, return result.
    pub async fn dispatch(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult {
        // Check if tool is allowed in current mode
        if !is_tool_allowed_in_mode(tool_name, ctx.mode) {
            return ToolResult::error(format!(
                "Tool '{tool_name}' is not available in plan mode. Only read-only tools are allowed."
            ));
        }

        // Look up tool
        let tool = match self.get(tool_name) {
            Some(t) => t,
            None => {
                return ToolResult::error(format!(
                    "Unknown tool: '{tool_name}'. Available tools: {}",
                    self.tool_names().join(", ")
                ));
            }
        };

        // Execute
        tool.execute(input, ctx).await
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a registry with all built-in tools.
pub fn create_default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    registry.register(Box::new(archon_tools::file_read::ReadTool));
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    registry.register(Box::new(archon_tools::file_edit::EditTool));
    registry.register(Box::new(archon_tools::glob_tool::GlobTool));
    registry.register(Box::new(archon_tools::grep::GrepTool));
    registry.register(Box::new(archon_tools::bash::BashTool::default()));
    registry.register(Box::new(archon_tools::powershell::PowerShellTool::default()));
    registry.register(Box::new(archon_tools::sleep::SleepTool));
    registry.register(Box::new(archon_tools::ask_user::AskUserTool));
    registry.register(Box::new(archon_tools::todo_write::TodoWriteTool));
    registry.register(Box::new(archon_tools::plan_mode::EnterPlanModeTool));
    registry.register(Box::new(archon_tools::plan_mode::ExitPlanModeTool));
    registry.register(Box::new(archon_tools::webfetch::WebFetchTool));
    registry.register(Box::new(archon_tools::config_tool::ConfigTool));
    registry.register(Box::new(archon_tools::agent_tool::AgentTool));
    registry.register(Box::new(archon_tools::send_message::SendMessageTool));
    registry.register(Box::new(archon_tools::notebook::NotebookEditTool));
    registry.register(Box::new(archon_tools::task_create::TaskCreateTool));
    registry.register(Box::new(archon_tools::task_get::TaskGetTool));
    registry.register(Box::new(archon_tools::task_update::TaskUpdateTool));
    registry.register(Box::new(archon_tools::task_list::TaskListTool));
    registry.register(Box::new(archon_tools::task_stop::TaskStopTool));
    registry.register(Box::new(archon_tools::task_output::TaskOutputTool));
    registry.register(Box::new(archon_tools::worktree::EnterWorktreeTool));
    registry.register(Box::new(archon_tools::worktree::ExitWorktreeTool));
    registry.register(Box::new(archon_tools::mcp_resources::ListMcpResourcesTool::default()));
    registry.register(Box::new(archon_tools::mcp_resources::ReadMcpResourceTool::default()));

    // ToolSearch needs the async ToolRegistry — register a placeholder here.
    // The actual ToolSearchTool is wired in at the session level where the
    // async ToolRegistry is available. See archon_tools::toolsearch.

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tools::tool::AgentMode;

    #[test]
    fn default_registry_has_all_tools() {
        let registry = create_default_registry();
        let names = registry.tool_names();

        assert!(names.contains(&"Read"), "missing Read tool");
        assert!(names.contains(&"Write"), "missing Write tool");
        assert!(names.contains(&"Edit"), "missing Edit tool");
        assert!(names.contains(&"Glob"), "missing Glob tool");
        assert!(names.contains(&"Grep"), "missing Grep tool");
        assert!(names.contains(&"Bash"), "missing Bash tool");
        assert!(names.contains(&"Sleep"), "missing Sleep tool");
        assert!(names.contains(&"TodoWrite"), "missing TodoWrite tool");
        assert!(names.contains(&"AskUserQuestion"), "missing AskUserQuestion");
        assert!(names.contains(&"EnterPlanMode"), "missing EnterPlanMode");
        assert!(names.contains(&"ExitPlanMode"), "missing ExitPlanMode");
        assert!(names.contains(&"WebFetch"), "missing WebFetch tool");
        assert!(names.contains(&"Config"), "missing Config tool");
        assert!(names.contains(&"Agent"), "missing Agent tool");
        assert!(names.contains(&"SendMessage"), "missing SendMessage tool");
        assert!(names.contains(&"NotebookEdit"), "missing NotebookEdit tool");
        assert!(names.contains(&"TaskCreate"), "missing TaskCreate tool");
        assert!(names.contains(&"TaskGet"), "missing TaskGet tool");
        assert!(names.contains(&"TaskUpdate"), "missing TaskUpdate tool");
        assert!(names.contains(&"TaskList"), "missing TaskList tool");
        assert!(names.contains(&"TaskStop"), "missing TaskStop tool");
        assert!(names.contains(&"TaskOutput"), "missing TaskOutput tool");
        assert!(names.contains(&"EnterWorktree"), "missing EnterWorktree tool");
        assert!(names.contains(&"ExitWorktree"), "missing ExitWorktree tool");
        assert!(names.contains(&"ListMcpResources"), "missing ListMcpResources tool");
        assert!(names.contains(&"ReadMcpResource"), "missing ReadMcpResource tool");
    }

    #[test]
    fn tool_definitions_valid_json() {
        let registry = create_default_registry();
        let defs = registry.tool_definitions();

        for def in &defs {
            assert!(def["name"].is_string(), "tool def missing name");
            assert!(def["description"].is_string(), "tool def missing description");
            assert!(def["input_schema"].is_object(), "tool def missing schema");
        }
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let registry = create_default_registry();
        let ctx = ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            mode: AgentMode::Normal,
        };

        let result = registry
            .dispatch("NonexistentTool", serde_json::json!({}), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn dispatch_blocked_in_plan_mode() {
        let registry = create_default_registry();
        let ctx = ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            mode: AgentMode::Plan,
        };

        let result = registry
            .dispatch("Write", serde_json::json!({}), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("plan mode"));
    }
}
