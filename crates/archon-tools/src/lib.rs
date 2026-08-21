pub(crate) mod agent_mutation_guard;
pub mod apply_patch;
pub mod ask_user;
// TASK-AGS-104: relocated from archon-core to break the
// archon-core <-> archon-tools dependency cycle. archon-core
// re-exports this module for back-compat so existing consumers
// keep the `archon_core::background_agents::*` path.
pub mod background_agents;
// TASK-TUI-402: thin shim API re-exports for TUI layer.
// TASK-TUI-406: spawn_gc_task added for registry memory bounds.
pub use background_agents::{
    PollOutcome, cancel_background_agent, poll_background_agent, spawn_gc_task,
};
pub mod bash;
pub mod bash_evidence;
mod authoritative_bash_execution_compile_contract {
    /// ```compile_fail
    /// use archon_tools::tool::AuthoritativeBashExecution;
    ///
    /// let forged = AuthoritativeBashExecution {
    ///     session_id: "session".into(),
    ///     tool_use_id: "tool".into(),
    ///     attempt: 0,
    ///     command: "cargo test".into(),
    ///     output: "test result: ok. 1 passed; 0 failed".into(),
    ///     exit_code: 0,
    /// };
    /// ```
    #[allow(dead_code)]
    pub struct ExternalCallersCannotForgeBashExecutions;
}
/// #201 Phase 3's acceptance criterion, as a pair: adding a tool without a
/// declared class must fail to compile, and adding one *with* a class must
/// still compile. Only the pair is meaningful — a `compile_fail` alone would
/// pass just as happily if the snippet failed for some unrelated reason, such
/// as `async_trait` not resolving in a doctest.
mod tool_capability_compile_contract {
    /// ```
    /// use archon_tools::tool::{PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult};
    ///
    /// struct DeclaredTool;
    ///
    /// #[async_trait::async_trait]
    /// impl Tool for DeclaredTool {
    ///     fn name(&self) -> &str { "Declared" }
    ///     fn description(&self) -> &str { "declares its class" }
    ///     fn input_schema(&self) -> serde_json::Value { serde_json::json!({}) }
    ///     fn capability(&self) -> ToolCapability { ToolCapability::HostLocal }
    ///     fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
    ///         PermissionLevel::Safe
    ///     }
    ///     async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
    ///         ToolResult::success("ok")
    ///     }
    /// }
    /// ```
    #[allow(dead_code)]
    pub struct ADeclaredToolCompiles;

    /// ```compile_fail
    /// use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
    ///
    /// struct UndeclaredTool;
    ///
    /// #[async_trait::async_trait]
    /// impl Tool for UndeclaredTool {
    ///     fn name(&self) -> &str { "Undeclared" }
    ///     fn description(&self) -> &str { "declares no class" }
    ///     fn input_schema(&self) -> serde_json::Value { serde_json::json!({}) }
    ///     fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
    ///         PermissionLevel::Safe
    ///     }
    ///     async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
    ///         ToolResult::success("ok")
    ///     }
    /// }
    /// ```
    #[allow(dead_code)]
    pub struct AToolWithoutACapabilityClassDoesNot;
}
pub(crate) mod bash_observability;
pub(crate) mod cargo_target_env;
pub use cargo_target_env::{current_timeout_exempt_cargo_wait, take_timeout_exempt_cargo_wait};
pub mod concurrency;
pub mod config_tool;
pub mod docs;
pub(crate) mod docs_runtime;
pub mod evidence_cli;
pub mod execution_deadline;
pub mod file_edit;
/// What each agent has actually looked at on disk (#193 Phase A).
pub mod file_observation;
pub mod file_read;
pub mod file_write;
/// The filesystem of the execution world (#201 Phase 1).
pub mod filesystem;
pub mod gametheory;
pub mod glob_tool;
pub mod grep;
pub mod large_edit;
pub mod monitor;
pub(crate) mod path_guard;
pub mod plan_mode;
pub mod plan_reconciliation;
pub mod plan_tasks;
pub mod powershell;
pub mod provider_env;
pub mod push_notification;
pub mod registry;
pub mod session_search;
pub mod sleep;
// Persistent shell sessions (#189 Phase 6). Only the tools are public; the
// buffer, the registry and the shell table are `pub(crate)` so an unwired
// helper still trips `dead_code` — a lesson from Phase 0 of the same issue.
pub(crate) mod terminal_buffer;
pub(crate) mod terminal_registry;
pub(crate) mod terminal_shell;
pub mod terminal_tools;
pub(crate) mod terminal_world;
pub mod todo_write;
pub mod tool;
#[cfg(test)]
#[path = "tool_capability_declaration_tests.rs"]
mod tool_capability_declaration_tests;

pub mod toolsearch;
pub mod webfetch;
// Public because `CargoResourceLimits` is a field on `BashTool`, which callers
// construct from their own config.
pub mod workflow_resource_env;
pub mod workflow_run_env;

pub mod agent_tool;
// TASK-AGS-105: SubagentExecutor trait + OnceLock registry. The
// concrete AgentSubagentExecutor is installed by archon-core at
// Agent::new time.
pub mod git;
pub mod send_message;
pub mod subagent_executor;
pub mod subagent_request;
pub mod validation;

pub mod task_create;
pub mod task_get;
pub mod task_list;
pub mod task_manager;
pub mod task_output;
pub mod task_stop;
pub mod task_update;

pub mod coordination_record;
pub mod isolation;
pub mod worktree;
pub mod worktree_disk;
pub mod worktree_exit;
pub mod worktree_manager;
pub mod worktree_ownership;
pub mod worktree_review;
pub mod write_claims;

pub mod board;
pub mod cron_create;
pub mod cron_delete;
pub mod cron_list;
pub mod cron_scheduler;
pub mod cron_shutdown;
pub mod cron_task;
pub mod mcp_resources;
pub mod memory;
pub mod verbosity_toggle;

pub mod remote_trigger;

pub mod lsp_client;
pub mod lsp_diagnostics;
pub mod lsp_formatters;
pub mod lsp_manager;
pub mod lsp_tool;
pub mod lsp_types;

pub mod team_config;
pub mod team_create;
pub mod team_delete;
pub mod team_message;
pub mod team_roster;

pub mod cartographer;
pub mod java;

pub mod leann_find_similar;
pub mod leann_search;
pub mod learning;
pub mod trading;

// Stubs for tools implemented in later tasks
pub mod agent {}
pub mod notebook;
pub mod web_search;
