pub mod apply_patch;
pub mod ask_user;
// TASK-AGS-104: relocated from archon-core to break the
// archon-core <-> archon-tools dependency cycle. archon-core
// re-exports this module for back-compat so existing consumers
// keep the `archon_core::background_agents::*` path.
pub mod background_agents;
// TASK-TUI-402: thin shim API re-exports for TUI layer.
// TASK-TUI-406: spawn_gc_task added for registry memory bounds.
pub use background_agents::{cancel_background_agent, poll_background_agent, spawn_gc_task, PollOutcome};
pub mod bash;
pub mod concurrency;
pub mod config_tool;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob_tool;
pub mod grep;
pub mod plan_mode;
pub mod powershell;
pub mod registry;
pub mod sleep;
pub mod todo_write;
pub mod tool;

pub mod toolsearch;
pub mod webfetch;

pub mod agent_tool;
// TASK-AGS-105: SubagentExecutor trait + OnceLock registry. The
// concrete AgentSubagentExecutor is installed by archon-core at
// Agent::new time.
pub mod subagent_executor;
pub mod git;
pub mod send_message;
pub mod validation;

pub mod task_create;
pub mod task_get;
pub mod task_list;
pub mod task_manager;
pub mod task_output;
pub mod task_stop;
pub mod task_update;

pub mod worktree;
pub mod worktree_manager;

pub mod cron_create;
pub mod cron_delete;
pub mod cron_list;
pub mod cron_scheduler;
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

pub mod team_backend;
pub mod team_config;
pub mod team_create;
pub mod team_delete;
pub mod team_message;

pub mod cartographer;

// Stubs for tools implemented in later tasks
pub mod agent {}
pub mod notebook;
pub mod web_search;
