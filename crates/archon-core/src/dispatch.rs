use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use archon_tools::plan_mode::is_tool_allowed_in_mode;
use archon_tools::tool::{Tool, ToolContext, ToolResult};

/// Registry of available tools.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
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
        self.tools.insert(name, Arc::from(tool));
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| &**t)
    }

    /// Get a cloneable handle to a tool for concurrent dispatch.
    pub fn lookup(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
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

    /// Create a new registry containing only the tools whose names appear
    /// in `allowed`. Arc pointers are cloned (cheap ref-count bump).
    /// An empty `allowed` list produces an empty registry.
    pub fn clone_filtered(&self, allowed: &[&str]) -> Self {
        let filtered = self
            .tools
            .iter()
            .filter(|(name, _)| allowed.contains(&name.as_str()))
            .map(|(name, tool)| (name.clone(), Arc::clone(tool)))
            .collect();
        Self { tools: filtered }
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
            // TASK-P0-B.3 (#174): append the intercepted call to
            // `.archon/plan.md` so the user can review it later via
            // `/plan` or edit via `/plan open`. IO failures are logged
            // but MUST NOT replace the block: the interception contract
            // (return an error so the model sees the tool failed) is
            // the primary behaviour; the plan-file append is an
            // additive audit trail.
            let plan_path = crate::plan_file::plan_path(&ctx.working_dir);
            if let Err(e) =
                crate::plan_file::append_plan_entry(&plan_path, tool_name, &input)
            {
                tracing::warn!(
                    error = %e,
                    plan_path = %plan_path.display(),
                    tool = tool_name,
                    "failed to append intercepted tool call to plan file"
                );
            }
            return ToolResult::error(format!(
                "Tool '{tool_name}' is not available in plan mode. Only read-only tools are allowed. \
                 The call has been queued in the plan file for review — use `/plan` to view or `/plan open` to edit."
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
///
/// `working_dir` is passed to tools that operate on the current project
/// (cron scheduler store, LSP manager, team config, etc.).
pub fn create_default_registry(working_dir: PathBuf) -> ToolRegistry {
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
    registry.register(Box::new(crate::skills::skill_tool::SkillTool));
    registry.register(Box::new(archon_tools::webfetch::WebFetchTool));
    registry.register(Box::new(archon_tools::config_tool::ConfigTool));
    registry.register(Box::new(archon_tools::agent_tool::AgentTool::new()));
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
    registry.register(Box::new(
        archon_tools::mcp_resources::ListMcpResourcesTool::default(),
    ));
    registry.register(Box::new(
        archon_tools::mcp_resources::ReadMcpResourceTool::default(),
    ));

    // ── Fix 3: 7 tools built but never registered (TASK-CLI-500) ─────────────
    registry.register(Box::new(archon_tools::cron_create::CronCreateTool::new(
        working_dir.clone(),
    )));
    registry.register(Box::new(archon_tools::cron_list::CronListTool::new(
        working_dir.clone(),
    )));
    registry.register(Box::new(archon_tools::cron_delete::CronDeleteTool::new(
        working_dir.clone(),
    )));
    registry.register(Box::new(archon_tools::team_create::TeamCreateTool::new(
        working_dir.clone(),
    )));
    registry.register(Box::new(archon_tools::team_delete::TeamDeleteTool::new(
        working_dir.clone(),
    )));
    {
        let lsp_manager = Arc::new(tokio::sync::Mutex::new(
            archon_tools::lsp_manager::LspServerManager::new(working_dir.clone(), None),
        ));
        registry.register(Box::new(archon_tools::lsp_tool::LspTool::new(lsp_manager)));
    }
    registry.register(Box::new(
        archon_tools::remote_trigger::RemoteTriggerTool::new(
            archon_tools::remote_trigger::RemoteTriggerConfig::default(),
        ),
    ));

    // Web search via DuckDuckGo.
    registry.register(Box::new(archon_tools::web_search::WebSearchTool));

    // Code Cartographer — symbol indexing and codebase navigation.
    registry.register(Box::new(archon_tools::cartographer::CartographerTool));

    // Register ToolSearch with a snapshot of all tool definitions captured at this point.
    // Must be registered LAST so the snapshot includes all other tools.
    let tool_defs_snapshot = registry.tool_definitions();
    registry.register(Box::new(archon_tools::toolsearch::ToolSearchTool::new(
        tool_defs_snapshot,
    )));

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tools::tool::AgentMode;

    #[test]
    fn default_registry_has_all_tools() {
        let working_dir = std::env::temp_dir();
        let registry = create_default_registry(working_dir);
        let names = registry.tool_names();

        // Core tools
        assert!(names.contains(&"Read"), "missing Read tool");
        assert!(names.contains(&"Write"), "missing Write tool");
        assert!(names.contains(&"Edit"), "missing Edit tool");
        assert!(names.contains(&"Glob"), "missing Glob tool");
        assert!(names.contains(&"Grep"), "missing Grep tool");
        assert!(names.contains(&"Bash"), "missing Bash tool");
        assert!(names.contains(&"Sleep"), "missing Sleep tool");
        assert!(names.contains(&"TodoWrite"), "missing TodoWrite tool");
        assert!(
            names.contains(&"AskUserQuestion"),
            "missing AskUserQuestion"
        );
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
        assert!(
            names.contains(&"EnterWorktree"),
            "missing EnterWorktree tool"
        );
        assert!(names.contains(&"ExitWorktree"), "missing ExitWorktree tool");
        assert!(
            names.contains(&"ListMcpResources"),
            "missing ListMcpResources tool"
        );
        assert!(
            names.contains(&"ReadMcpResource"),
            "missing ReadMcpResource tool"
        );

        // TASK-CLI-500 Fix 3: previously missing tools now registered
        assert!(
            names.contains(&"CronCreate"),
            "missing CronCreate tool (Fix 3)"
        );
        assert!(names.contains(&"CronList"), "missing CronList tool (Fix 3)");
        assert!(
            names.contains(&"CronDelete"),
            "missing CronDelete tool (Fix 3)"
        );
        assert!(
            names.contains(&"TeamCreate"),
            "missing TeamCreate tool (Fix 3)"
        );
        assert!(
            names.contains(&"TeamDelete"),
            "missing TeamDelete tool (Fix 3)"
        );
        assert!(names.contains(&"lsp"), "missing LSP tool (Fix 3)");
        assert!(
            names.contains(&"RemoteTrigger"),
            "missing RemoteTrigger tool (Fix 3)"
        );

        // TASK-CLI-410: Code Cartographer
        assert!(
            names.contains(&"CartographerScan"),
            "missing CartographerScan tool (TASK-CLI-410)"
        );
        assert!(names.contains(&"ToolSearch"), "missing ToolSearch tool");
    }

    #[test]
    fn tool_definitions_valid_json() {
        let registry = create_default_registry(std::env::temp_dir());
        let defs = registry.tool_definitions();

        for def in &defs {
            assert!(def["name"].is_string(), "tool def missing name");
            assert!(
                def["description"].is_string(),
                "tool def missing description"
            );
            assert!(def["input_schema"].is_object(), "tool def missing schema");
        }
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let registry = create_default_registry(std::env::temp_dir());
        let ctx = ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            mode: AgentMode::Normal,
            extra_dirs: vec![],
            ..Default::default()
        };

        let result = registry
            .dispatch("NonexistentTool", serde_json::json!({}), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[test]
    fn clone_filtered_with_subset() {
        let registry = create_default_registry(std::env::temp_dir());
        let filtered = registry.clone_filtered(&["Read", "Grep"]);
        let names = filtered.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Grep"));
    }

    #[test]
    fn clone_filtered_empty_list_returns_empty() {
        let registry = create_default_registry(std::env::temp_dir());
        let filtered = registry.clone_filtered(&[]);
        assert!(filtered.tool_names().is_empty());
    }

    #[test]
    fn clone_filtered_nonexistent_tool_ignored() {
        let registry = create_default_registry(std::env::temp_dir());
        let filtered = registry.clone_filtered(&["Read", "FakeTool"]);
        let names = filtered.tool_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"Read"));
    }

    #[test]
    fn clone_filtered_does_not_mutate_original() {
        let registry = create_default_registry(std::env::temp_dir());
        let original_count = registry.tool_names().len();
        let _filtered = registry.clone_filtered(&["Read"]);
        assert_eq!(registry.tool_names().len(), original_count);
    }

    #[test]
    fn clone_filtered_tool_definitions_match() {
        let registry = create_default_registry(std::env::temp_dir());
        let filtered = registry.clone_filtered(&["Read", "Glob"]);
        let defs = filtered.tool_definitions();
        assert_eq!(defs.len(), 2);
        let def_names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert!(def_names.contains(&"Read"));
        assert!(def_names.contains(&"Glob"));
    }

    #[tokio::test]
    async fn dispatch_blocked_in_plan_mode() {
        let registry = create_default_registry(std::env::temp_dir());
        let ctx = ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            mode: AgentMode::Plan,
            extra_dirs: vec![],
            ..Default::default()
        };

        let result = registry
            .dispatch("Write", serde_json::json!({}), &ctx)
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("plan mode"));
    }
}
