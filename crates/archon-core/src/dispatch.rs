use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_observability::{AgentActivityEvent, AgentActivityKind, AgentActivityStatus};
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
        if self.tools.contains_key(&name) {
            tracing::warn!(tool = %name, "skipping duplicate tool registration");
            return;
        }
        self.tools.insert(name, Arc::from(tool));
    }

    /// Replace an existing tool registration or insert it if absent.
    pub fn replace(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            tracing::debug!(tool = %name, "replacing tool registration");
        }
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
        let mut tools: Vec<_> = self.tools.iter().collect();
        tools.sort_by(|(left, _), (right, _)| left.cmp(right));
        tools
            .into_iter()
            .map(|(_, tool)| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema(),
                })
            })
            .collect()
    }

    /// Dispatch a tool call: check mode, check sandbox, execute, return result.
    pub async fn dispatch(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult {
        // GHOST-006: sandbox pre-check (subagent path — BOTH dispatch sites must gate).
        if let Some(ref backend) = ctx.sandbox
            && let Err(reason) = backend.check(tool_name, &input)
        {
            emit_tool_activity(
                ctx,
                tool_name,
                AgentActivityKind::ToolFailed,
                AgentActivityStatus::Failed,
            );
            return ToolResult::error(reason);
        }

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
            if let Err(e) = crate::plan_file::append_plan_entry(&plan_path, tool_name, &input) {
                tracing::warn!(
                    error = %e,
                    plan_path = %plan_path.display(),
                    tool = tool_name,
                    "failed to append intercepted tool call to plan file"
                );
            }
            emit_tool_activity(
                ctx,
                tool_name,
                AgentActivityKind::ToolFailed,
                AgentActivityStatus::Failed,
            );
            return ToolResult::error(format!(
                "Tool '{tool_name}' is not available in plan mode. Only read-only tools are allowed. \
                 The call has been queued in the plan file for review — use `/plan` to view or `/plan open` to edit."
            ));
        }

        // Look up tool
        let tool = match self.get(tool_name) {
            Some(t) => t,
            None => {
                emit_tool_activity(
                    ctx,
                    tool_name,
                    AgentActivityKind::ToolFailed,
                    AgentActivityStatus::Failed,
                );
                return ToolResult::error(format!(
                    "Unknown tool: '{tool_name}'. Available tools: {}",
                    self.tool_names().join(", ")
                ));
            }
        };

        // Execute
        emit_tool_activity(
            ctx,
            tool_name,
            AgentActivityKind::ToolStarted,
            AgentActivityStatus::Running,
        );
        let started_at = Instant::now();
        let result = tool.execute(input, ctx).await;
        if result.is_error {
            emit_tool_activity_with_elapsed(
                ctx,
                tool_name,
                AgentActivityKind::ToolFailed,
                AgentActivityStatus::Failed,
                Some(started_at.elapsed()),
            );
        } else {
            emit_tool_activity_with_elapsed(
                ctx,
                tool_name,
                AgentActivityKind::ToolCompleted,
                AgentActivityStatus::Completed,
                Some(started_at.elapsed()),
            );
        }
        result
    }
}

pub(crate) fn emit_tool_activity(
    ctx: &ToolContext,
    tool_name: &str,
    kind: AgentActivityKind,
    status: AgentActivityStatus,
) {
    emit_tool_activity_with_elapsed(ctx, tool_name, kind, status, None);
}

pub(crate) fn emit_tool_activity_with_elapsed(
    ctx: &ToolContext,
    tool_name: &str,
    kind: AgentActivityKind,
    status: AgentActivityStatus,
    elapsed: Option<Duration>,
) {
    if let Some(sink) = &ctx.activity_sink {
        let message = match elapsed {
            Some(elapsed) => format!("{tool_name} elapsed={}", format_duration(elapsed)),
            None => tool_name.to_string(),
        };
        sink.emit(AgentActivityEvent::new(
            ctx.session_id.clone(),
            kind,
            status,
            message,
        ));
    }
}

fn format_duration(elapsed: Duration) -> String {
    let millis = elapsed.as_millis();
    if millis < 1_000 {
        format!("{millis}ms")
    } else {
        format!("{:.1}s", elapsed.as_secs_f64())
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
pub fn create_default_registry(
    working_dir: PathBuf,
    leann_index: Option<std::sync::Arc<archon_leann::CodeIndex>>,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    registry.register(Box::new(archon_tools::file_read::ReadTool));
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    registry.register(Box::new(archon_tools::file_edit::EditTool));
    registry.register(Box::new(archon_tools::large_edit::LargeEditBeginTool));
    registry.register(Box::new(archon_tools::large_edit::LargeEditInsertAfterTool));
    registry.register(Box::new(
        archon_tools::large_edit::LargeEditReplaceSectionTool,
    ));
    registry.register(Box::new(
        archon_tools::large_edit::LargeEditDeleteSectionTool,
    ));
    registry.register(Box::new(archon_tools::large_edit::LargeEditCommitTool));
    registry.register(Box::new(archon_tools::large_edit::LargeEditAbortTool));
    // TASK-P0-B.5 (#183): ApplyPatch registered next to EditTool for
    // topical locality — both are filesystem-mutating edit tools.
    registry.register(Box::new(archon_tools::apply_patch::ApplyPatchTool));
    registry.register(Box::new(archon_tools::glob_tool::GlobTool));
    registry.register(Box::new(archon_tools::grep::GrepTool));
    registry.register(Box::new(archon_tools::bash::BashTool::default()));
    // TASK-P0-B.6a (#184): Monitor registered next to Bash for topical
    // locality — both spawn shell commands; Monitor differs by returning
    // bounded-time stdout events instead of blocking until exit.
    registry.register(Box::new(archon_tools::monitor::MonitorTool));
    // TASK-P0-B.6b (#185): PushNotification emits a structured
    // tracing event on the `archon::notification` target. Registered
    // alongside Monitor because both are "observability" tools —
    // Monitor observes external commands, PushNotification lets the
    // LLM surface events of its own.
    registry.register(Box::new(
        archon_tools::push_notification::PushNotificationTool,
    ));
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

    // Evidence Engine document-intelligence tool surface. These tools execute
    // the same CLI command paths users exercise, then return the observed
    // command output to the agent.
    registry.register(Box::new(archon_tools::docs::DocIngest));
    registry.register(Box::new(archon_tools::docs::DocList));
    registry.register(Box::new(archon_tools::docs::DocGet));
    registry.register(Box::new(archon_tools::docs::DocStatus));
    registry.register(Box::new(archon_tools::docs::DocSearch));
    registry.register(Box::new(archon_tools::docs::DocAnswer));
    registry.register(Box::new(archon_tools::docs::DocProvenance));
    registry.register(Box::new(archon_tools::docs::DocInspect));
    registry.register(Box::new(archon_tools::docs::DocModelStatus));

    // Game-theory evidence engine tool surface. The concrete executor is
    // installed by the binary layer to avoid archon-tools -> archon-pipeline
    // dependency cycles.
    registry.register(Box::new(archon_tools::gametheory::GameTheoryRun));
    registry.register(Box::new(archon_tools::gametheory::GameTheoryStatus));
    registry.register(Box::new(archon_tools::gametheory::GameTheoryListAgents));
    registry.register(Box::new(archon_tools::gametheory::GameTheorySpecimens));
    registry.register(Box::new(archon_tools::gametheory::GameTheoryInspect));
    registry.register(Box::new(archon_tools::gametheory::GameTheoryReplay));
    registry.register(Box::new(archon_tools::gametheory::GameTheoryClassify));
    registry.register(Box::new(archon_tools::gametheory::GameTheoryCallSpecialist));

    // Governed-learning tool surface required by the Evidence Engine TSPEC.
    registry.register(Box::new(archon_tools::learning::LearningStatus));
    registry.register(Box::new(archon_tools::learning::LearningInspect));
    registry.register(Box::new(archon_tools::learning::BehaviourProposals));
    registry.register(Box::new(archon_tools::learning::BehaviourApprove));
    registry.register(Box::new(archon_tools::learning::BehaviourRollback));

    // Code Cartographer — symbol indexing and codebase navigation.
    registry.register(Box::new(archon_tools::cartographer::CartographerTool));

    // LEANN semantic code search — only registered when the index is
    // available (graceful no-op when LEANN initialisation fails).
    if let Some(ref idx) = leann_index {
        registry.register(Box::new(archon_tools::leann_search::LeannSearchTool::new(
            std::sync::Arc::clone(idx),
        )));
        registry.register(Box::new(
            archon_tools::leann_find_similar::LeannFindSimilarTool::new(std::sync::Arc::clone(idx)),
        ));
    }

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
    use archon_observability::{AgentActivityKind, InMemoryActivitySink};
    use archon_tools::tool::{AgentMode, PermissionLevel};

    struct ReplaceTestTool(&'static str);

    #[async_trait::async_trait]
    impl Tool for ReplaceTestTool {
        fn name(&self) -> &str {
            "ReplaceTest"
        }

        fn description(&self) -> &str {
            self.0
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::success(self.0)
        }

        fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
            PermissionLevel::Safe
        }
    }

    #[test]
    fn replace_overwrites_existing_tool_registration() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ReplaceTestTool("first")));
        registry.replace(Box::new(ReplaceTestTool("second")));

        let definition = registry
            .tool_definitions()
            .into_iter()
            .find(|tool| tool["name"] == "ReplaceTest")
            .expect("replacement tool should be registered");
        assert_eq!(definition["description"], "second");
    }

    #[test]
    fn default_registry_has_all_tools() {
        let working_dir = std::env::temp_dir();
        let registry = create_default_registry(working_dir, None);
        let names = registry.tool_names();

        // Core tools
        assert!(names.contains(&"Read"), "missing Read tool");
        assert!(names.contains(&"Write"), "missing Write tool");
        assert!(names.contains(&"Edit"), "missing Edit tool");
        assert!(names.contains(&"LargeEditBegin"), "missing LargeEditBegin");
        assert!(
            names.contains(&"LargeEditInsertAfter"),
            "missing LargeEditInsertAfter"
        );
        assert!(
            names.contains(&"LargeEditReplaceSection"),
            "missing LargeEditReplaceSection"
        );
        assert!(
            names.contains(&"LargeEditDeleteSection"),
            "missing LargeEditDeleteSection"
        );
        assert!(
            names.contains(&"LargeEditCommit"),
            "missing LargeEditCommit"
        );
        assert!(names.contains(&"LargeEditAbort"), "missing LargeEditAbort");
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
        for name in archon_tools::gametheory::GAMETHEORY_TOOL_NAMES {
            assert!(names.contains(name), "missing {name} tool (Group 9)");
        }
        for name in archon_tools::docs::DOC_TOOL_NAMES {
            assert!(names.contains(name), "missing {name} tool (TSPEC §12)");
        }
        for name in archon_tools::learning::LEARNING_TOOL_NAMES {
            assert!(names.contains(name), "missing {name} tool (TSPEC §12)");
        }
        assert!(names.contains(&"ToolSearch"), "missing ToolSearch tool");
    }

    #[test]
    fn test_all_8_gametheory_tools_registered() {
        let registry = create_default_registry(std::env::temp_dir(), None);
        let names = registry.tool_names();
        let registered: Vec<_> = archon_tools::gametheory::GAMETHEORY_TOOL_NAMES
            .iter()
            .filter(|name| names.contains(name))
            .copied()
            .collect();

        assert_eq!(
            registered,
            archon_tools::gametheory::GAMETHEORY_TOOL_NAMES,
            "all Group 9 gametheory tools must be discoverable from the runtime registry"
        );
    }

    #[test]
    fn test_evidence_engine_tools_registered() {
        let registry = create_default_registry(std::env::temp_dir(), None);
        let names = registry.tool_names();

        for name in archon_tools::docs::DOC_TOOL_NAMES {
            assert!(
                names.contains(name),
                "Doc tool {name} must be discoverable from the runtime registry"
            );
        }
        for name in archon_tools::learning::LEARNING_TOOL_NAMES {
            assert!(
                names.contains(name),
                "Learning tool {name} must be discoverable from the runtime registry"
            );
        }
    }

    #[test]
    fn tool_definitions_valid_json() {
        let registry = create_default_registry(std::env::temp_dir(), None);
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

    #[test]
    fn docs_do_not_reference_unknown_tools() {
        let documented = documented_tool_names();
        let registry = create_default_registry(std::env::temp_dir(), None);
        let mut registered: std::collections::HashSet<String> = registry
            .tool_names()
            .into_iter()
            .map(str::to_string)
            .collect();

        // Session-wired tools need runtime dependencies (the memory graph);
        // they are still real tools and are registered by src/session.rs.
        registered.insert("memory_store".to_string());
        registered.insert("memory_recall".to_string());

        // LEANN tools are conditional because they require an available index
        // at startup. The docs explicitly mark them as conditional.
        let conditional = ["LeannSearch", "LeannFindSimilar"];

        let unknown: Vec<_> = documented
            .into_iter()
            .filter(|name| !registered.contains(name) && !conditional.contains(&name.as_str()))
            .collect();

        assert!(
            unknown.is_empty(),
            "docs/reference/tools.md references unknown tools: {unknown:?}"
        );
    }

    fn documented_tool_names() -> Vec<String> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/tools.md");
        let markdown = std::fs::read_to_string(path).expect("tool docs exist");

        markdown
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                if !trimmed.starts_with("| `") {
                    return None;
                }
                let cell = trimmed.split('|').nth(1)?;
                let start = cell.find('`')? + 1;
                let rest = &cell[start..];
                let end = rest.find('`')?;
                Some(rest[..end].to_string())
            })
            .filter(|name| name != "Tool")
            .collect()
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let registry = create_default_registry(std::env::temp_dir(), None);
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

    #[tokio::test]
    async fn dispatch_success_emits_started_and_completed_activity() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ActivityTestTool::success("ActivityEcho")));
        let sink = Arc::new(InMemoryActivitySink::new());
        let ctx = activity_ctx(sink.clone());

        let result = registry
            .dispatch("ActivityEcho", serde_json::json!({}), &ctx)
            .await;

        assert!(!result.is_error);
        let events = sink.events();
        let kinds: Vec<_> = events.iter().map(|event| event.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AgentActivityKind::ToolStarted,
                AgentActivityKind::ToolCompleted
            ]
        );
        assert_eq!(events[0].message, "ActivityEcho");
        assert!(events[1].message.starts_with("ActivityEcho elapsed="));
        assert!(
            events
                .iter()
                .all(|event| event.session_id == "activity-test")
        );
    }

    #[tokio::test]
    async fn dispatch_tool_error_emits_started_and_failed_activity() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ActivityTestTool::failure("ActivityFail")));
        let sink = Arc::new(InMemoryActivitySink::new());
        let ctx = activity_ctx(sink.clone());

        let result = registry
            .dispatch("ActivityFail", serde_json::json!({}), &ctx)
            .await;

        assert!(result.is_error);
        let kinds: Vec<_> = sink.events().iter().map(|event| event.kind).collect();
        assert_eq!(
            kinds,
            vec![
                AgentActivityKind::ToolStarted,
                AgentActivityKind::ToolFailed
            ]
        );
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_emits_failed_activity() {
        let registry = ToolRegistry::new();
        let sink = Arc::new(InMemoryActivitySink::new());
        let ctx = activity_ctx(sink.clone());

        let result = registry
            .dispatch("MissingActivityTool", serde_json::json!({}), &ctx)
            .await;

        assert!(result.is_error);
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AgentActivityKind::ToolFailed);
        assert_eq!(events[0].message, "MissingActivityTool");
    }

    #[test]
    fn clone_filtered_with_subset() {
        let registry = create_default_registry(std::env::temp_dir(), None);
        let filtered = registry.clone_filtered(&["Read", "Grep"]);
        let names = filtered.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Grep"));
    }

    #[test]
    fn clone_filtered_empty_list_returns_empty() {
        let registry = create_default_registry(std::env::temp_dir(), None);
        let filtered = registry.clone_filtered(&[]);
        assert!(filtered.tool_names().is_empty());
    }

    #[test]
    fn clone_filtered_nonexistent_tool_ignored() {
        let registry = create_default_registry(std::env::temp_dir(), None);
        let filtered = registry.clone_filtered(&["Read", "FakeTool"]);
        let names = filtered.tool_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"Read"));
    }

    #[test]
    fn clone_filtered_does_not_mutate_original() {
        let registry = create_default_registry(std::env::temp_dir(), None);
        let original_count = registry.tool_names().len();
        let _filtered = registry.clone_filtered(&["Read"]);
        assert_eq!(registry.tool_names().len(), original_count);
    }

    #[test]
    fn clone_filtered_tool_definitions_match() {
        let registry = create_default_registry(std::env::temp_dir(), None);
        let filtered = registry.clone_filtered(&["Read", "Glob"]);
        let defs = filtered.tool_definitions();
        assert_eq!(defs.len(), 2);
        let def_names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert!(def_names.contains(&"Read"));
        assert!(def_names.contains(&"Glob"));
    }

    #[tokio::test]
    async fn dispatch_blocked_in_plan_mode() {
        let registry = create_default_registry(std::env::temp_dir(), None);
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

    fn activity_ctx(sink: Arc<InMemoryActivitySink>) -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "activity-test".to_string(),
            activity_sink: Some(sink),
            ..Default::default()
        }
    }

    struct ActivityTestTool {
        name: &'static str,
        succeeds: bool,
    }

    impl ActivityTestTool {
        fn success(name: &'static str) -> Self {
            Self {
                name,
                succeeds: true,
            }
        }

        fn failure(name: &'static str) -> Self {
            Self {
                name,
                succeeds: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl Tool for ActivityTestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "activity test tool"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            if self.succeeds {
                ToolResult::success("ok")
            } else {
                ToolResult::error("failed")
            }
        }

        fn permission_level(
            &self,
            _input: &serde_json::Value,
        ) -> archon_tools::tool::PermissionLevel {
            archon_tools::tool::PermissionLevel::Safe
        }
    }
}
