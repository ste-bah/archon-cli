use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use archon_observability::{AgentActivityEvent, AgentActivityKind, AgentActivityStatus};
use archon_tools::plan_mode::is_tool_allowed_in_mode;
#[cfg(test)]
use archon_tools::tool::WorkingTreeEffect;
use archon_tools::tool::{Tool, ToolContext, ToolResult};

/// Reviewed effects for every built-in registered below. The exact-set test in
/// `dispatch_registry_tests` fails until a newly registered tool is reviewed.
#[cfg(test)]
pub(crate) const PRODUCTION_TOOL_EFFECTS: &[(&str, WorkingTreeEffect)] = &[
    ("Agent", WorkingTreeEffect::Arbitrary),
    ("ApplyPatch", WorkingTreeEffect::DeclaredPaths),
    ("AskUserQuestion", WorkingTreeEffect::ExternalOnly),
    ("Bash", WorkingTreeEffect::Arbitrary),
    ("BehaviourApprove", WorkingTreeEffect::Arbitrary),
    ("BehaviourProposals", WorkingTreeEffect::Arbitrary),
    ("BehaviourRollback", WorkingTreeEffect::Arbitrary),
    ("BoardClaim", WorkingTreeEffect::ExternalOnly),
    ("BoardList", WorkingTreeEffect::ExternalOnly),
    ("BoardRaise", WorkingTreeEffect::ExternalOnly),
    ("BoardResolve", WorkingTreeEffect::ExternalOnly),
    ("CartographerScan", WorkingTreeEffect::ExternalOnly),
    ("Config", WorkingTreeEffect::ExternalOnly),
    ("CronCreate", WorkingTreeEffect::Arbitrary),
    ("CronDelete", WorkingTreeEffect::Arbitrary),
    ("CronList", WorkingTreeEffect::None),
    ("DocAnswer", WorkingTreeEffect::Arbitrary),
    ("DocGet", WorkingTreeEffect::Arbitrary),
    ("DocIngest", WorkingTreeEffect::Arbitrary),
    ("DocInspect", WorkingTreeEffect::Arbitrary),
    ("DocList", WorkingTreeEffect::Arbitrary),
    ("DocModelStatus", WorkingTreeEffect::Arbitrary),
    ("DocProvenance", WorkingTreeEffect::Arbitrary),
    ("DocSearch", WorkingTreeEffect::Arbitrary),
    ("DocStatus", WorkingTreeEffect::Arbitrary),
    ("Edit", WorkingTreeEffect::DeclaredPaths),
    ("EnterPlanMode", WorkingTreeEffect::ExternalOnly),
    ("EnterWorktree", WorkingTreeEffect::Arbitrary),
    ("ExitPlanMode", WorkingTreeEffect::ExternalOnly),
    ("ExitWorktree", WorkingTreeEffect::Arbitrary),
    ("GameTheoryCallSpecialist", WorkingTreeEffect::Arbitrary),
    ("GameTheoryClassify", WorkingTreeEffect::Arbitrary),
    ("GameTheoryInspect", WorkingTreeEffect::Arbitrary),
    ("GameTheoryListAgents", WorkingTreeEffect::None),
    ("GameTheoryReplay", WorkingTreeEffect::Arbitrary),
    ("GameTheoryRun", WorkingTreeEffect::Arbitrary),
    ("GameTheorySpecimens", WorkingTreeEffect::Arbitrary),
    ("GameTheoryStatus", WorkingTreeEffect::Arbitrary),
    ("Glob", WorkingTreeEffect::None),
    ("Grep", WorkingTreeEffect::None),
    ("JavaToolchain", WorkingTreeEffect::Arbitrary),
    ("LargeEditAbort", WorkingTreeEffect::Arbitrary),
    ("LargeEditBegin", WorkingTreeEffect::Arbitrary),
    ("LargeEditCommit", WorkingTreeEffect::Arbitrary),
    ("LargeEditDeleteSection", WorkingTreeEffect::Arbitrary),
    ("LargeEditInsertAfter", WorkingTreeEffect::Arbitrary),
    ("LargeEditReplaceSection", WorkingTreeEffect::Arbitrary),
    ("LearningInspect", WorkingTreeEffect::Arbitrary),
    ("LearningStatus", WorkingTreeEffect::Arbitrary),
    ("ListMcpResources", WorkingTreeEffect::ExternalOnly),
    ("Monitor", WorkingTreeEffect::Arbitrary),
    ("NotebookEdit", WorkingTreeEffect::DeclaredPaths),
    ("PowerShell", WorkingTreeEffect::Arbitrary),
    ("PushNotification", WorkingTreeEffect::ExternalOnly),
    ("Read", WorkingTreeEffect::None),
    ("ReadMcpResource", WorkingTreeEffect::ExternalOnly),
    ("RemoteTrigger", WorkingTreeEffect::ExternalOnly),
    ("SendMessage", WorkingTreeEffect::ExternalOnly),
    // Reads the local session store and writes nothing (#189 Phase 2).
    ("SessionSearch", WorkingTreeEffect::None),
    ("Skill", WorkingTreeEffect::Arbitrary),
    ("Sleep", WorkingTreeEffect::None),
    ("TaskCreate", WorkingTreeEffect::Arbitrary),
    ("TaskGet", WorkingTreeEffect::None),
    ("TaskList", WorkingTreeEffect::None),
    ("TaskOutput", WorkingTreeEffect::None),
    ("TaskStop", WorkingTreeEffect::ExternalOnly),
    ("TaskUpdate", WorkingTreeEffect::ExternalOnly),
    ("TeamCreate", WorkingTreeEffect::Arbitrary),
    ("TeamDelete", WorkingTreeEffect::Arbitrary),
    ("TodoWrite", WorkingTreeEffect::ExternalOnly),
    ("ToolSearch", WorkingTreeEffect::None),
    ("WebFetch", WorkingTreeEffect::ExternalOnly),
    ("WebSearch", WorkingTreeEffect::ExternalOnly),
    ("Write", WorkingTreeEffect::DeclaredPaths),
    ("lsp", WorkingTreeEffect::Arbitrary),
];

/// Tools no allowlist removes: how an agent talks, and how it waits.
///
/// These are offered to every agent and every subagent however its toolset was
/// derived. The shared reason is that withholding one does not restrict what an
/// agent can **do** — it removes its ability to say what it found, reach the
/// agent that spawned it, or wait for something it depends on. An agent that
/// cannot answer is not a teammate, and a coordination layer nothing can invoke
/// is machinery pretending to be a feature (#153, #184).
///
/// This is an always-OFFER set, not an override of a deliberate refusal:
/// [`ToolRegistry::filter_blacklist`] and the subagent denylist both still win.
pub const ALWAYS_AVAILABLE_TOOLS: &[&str] = &[
    // The board: how a subagent hands work back.
    "BoardRaise",
    "BoardClaim",
    "BoardList",
    "BoardResolve",
    // The router: how it reaches its lead and its peers.
    "SendMessage",
    // Waiting: an agent coordinating with another has to be able to wait for
    // it. Without this the only way to pause is to burn a tool round, or to
    // shell out to a sleep command it may not have.
    "Sleep",
];

/// Whether `name` is in [`ALWAYS_AVAILABLE_TOOLS`].
pub fn is_always_available(name: &str) -> bool {
    ALWAYS_AVAILABLE_TOOLS.contains(&name)
}

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

    /// Restrict this registry's `Bash` to the agent's isolation tier (#184 M3).
    ///
    /// The registry is built per subagent, which makes it the one place a
    /// per-agent restriction can live: `ToolContext` is inherited verbatim from
    /// the parent, and the admission callback is skipped for `Safe` tools.
    ///
    /// Returns `false` when there is no `Bash` to restrict — a read-only agent
    /// whose allowlist excluded it, which needs no restriction anyway.
    pub fn set_bash_isolation_tier(
        &mut self,
        tier: archon_tools::isolation::IsolationTier,
    ) -> bool {
        // Nothing to enforce, and rebuilding the tool would discard whatever
        // provider-env configuration was attached moments earlier.
        if tier.may_build() {
            return true;
        }
        let Some(bash) = self.tools.get("Bash") else {
            return false;
        };
        let Some(restricted) = bash.with_isolation_tier(tier) else {
            return false;
        };
        self.replace(restricted);
        true
    }

    pub fn attach_provider_env_to_bash(
        &mut self,
        provider_env: archon_tools::provider_env::ProviderEnvSource,
    ) -> bool {
        let Some(bash) = self.tools.get("Bash") else {
            return false;
        };
        let Some(configured) = bash.with_provider_env_source(provider_env) else {
            return false;
        };
        self.replace(configured);
        true
    }

    /// Get all tool names.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Keep only the tools whose names appear in the whitelist, plus
    /// [`ALWAYS_AVAILABLE_TOOLS`].
    ///
    /// The retention is here rather than at each caller because the caller is
    /// the wrong place to remember it. A whitelist applied at session start
    /// *deletes* tools from the registry, and a subagent's toolset is later
    /// taken from that same registry by name — so a tool dropped here could not
    /// be restored downstream however loudly the spawn asked for it. The
    /// subagent path unions the same names into its request and would silently
    /// get nothing back, because an intersection cannot produce what the source
    /// no longer holds.
    ///
    /// Use [`Self::filter_blacklist`] to refuse one of these deliberately;
    /// a denial still wins.
    pub fn filter_whitelist(&mut self, names: &[&str]) {
        self.tools
            .retain(|k, _| names.contains(&k.as_str()) || is_always_available(k));
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
        tools.sort_by_key(|(left, _)| *left);
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
        // Check if tool is allowed in current mode
        if !is_tool_allowed_in_mode(tool_name, ctx.mode) {
            // Append the intercepted call to the session-scoped immutable audit
            // log. The editable document remains separate and is only opened by
            // `/plan open`. IO failures are logged but MUST NOT replace the
            // block: returning an error so the model sees the tool failed is the
            // primary behaviour; the audit append is additive.
            match crate::plan_file::plan_audit_path(&ctx.working_dir, &ctx.session_id) {
                Ok(audit_path) => {
                    if let Err(error) =
                        crate::plan_file::append_plan_entry(&audit_path, tool_name, &input)
                    {
                        tracing::warn!(
                            error = %error,
                            audit_path = %audit_path.display(),
                            tool = tool_name,
                            "failed to append intercepted tool call to session audit log"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    error = %error,
                    session_id = %ctx.session_id,
                    tool = tool_name,
                    "refused unsafe session ID for Plan Mode audit log"
                ),
            }
            emit_tool_activity(
                ctx,
                tool_name,
                AgentActivityKind::ToolFailed,
                AgentActivityStatus::Failed,
            );
            return ToolResult::error(format!(
                "Tool '{tool_name}' is not available in Plan Mode. Plan Mode blocks working-tree mutations by default; only the canonical Plan-safe allowlist is available, including TaskCreate, TaskUpdate, and Agent. The call has been recorded in the session audit for review."
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
        crate::tool_run_admission::execute_tool_attempt(tool, input, ctx, false).await
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
    // #189 Phase 2: session search was reachable only by typing /sessions.
    // The configured path is resolved here because `archon-tools` cannot see
    // `ArchonConfig`, and a tool guessing would mean searching a different
    // database than `/sessions` reads.
    registry.register(Box::new(
        archon_tools::session_search::SessionSearchTool::new(
            crate::config::load_config()
                .ok()
                .and_then(|loaded| loaded.session.db_path.map(PathBuf::from)),
        ),
    ));
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
    // Task board. Registered unconditionally: the tools resolve the board
    // handle at call time, not here, because `create_default_registry` runs
    // before the memory service is opened and in ~25 places that never open one
    // at all. Without a handle they return "the task board is unavailable"
    // rather than being silently absent from the tool list — an agent told to
    // drain the board can then say why it could not.
    registry.register(Box::new(archon_tools::board::BoardRaiseTool::new()));
    registry.register(Box::new(archon_tools::board::BoardClaimTool::new()));
    registry.register(Box::new(archon_tools::board::BoardListTool::new()));
    registry.register(Box::new(archon_tools::board::BoardResolveTool::new()));
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

    // Java build-and-analysis toolchain. Registered unconditionally: it
    // detects the build system per invocation and says so when a directory is
    // not a Java project, which is a more useful answer than the tool being
    // absent from the surface entirely.
    registry.register(Box::new(archon_tools::java::JavaToolchain));

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
#[path = "dispatch_registry_tests.rs"]
mod registry_tests;
#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "dispatch_tool_run_tests.rs"]
mod tool_run_tests;
