pub mod metrics;
pub mod agent;
pub mod agents;
pub mod archonmd;
pub use metrics::ChannelMetricSink;
// TASK-AGS-101/104: BackgroundAgentRegistry. The module was relocated
// to archon-tools in TASK-AGS-104 to break the
// archon-core <-> archon-tools dependency cycle. Re-exported here so
// existing `archon_core::background_agents::*` paths keep working.
pub use archon_tools::background_agents;
pub use archon_tools::background_agents::{
    AgentStatus, BackgroundAgentHandle, BackgroundAgentRegistry, BackgroundAgentRegistryApi,
    RegistryError, RegistryEvent, BACKGROUND_AGENTS,
};
pub mod cli_flags;
pub mod commands;
pub mod config;
pub mod config_diff;
pub mod config_layers;
pub mod config_source;
pub mod config_watcher;
pub mod cost;
pub mod cost_alerts;
pub mod dispatch;
pub mod env_vars;
pub mod git;
pub mod headless;
pub mod hooks;
pub mod input_format;
pub mod logging;
pub mod orchestrator;
pub mod output_format;
pub mod output_style;
pub mod patterns;
pub mod output_style_loader;
pub mod print_mode;
pub mod reasoning;
pub mod remote;
pub mod schema_validation;
pub mod skills;
pub mod subagent;
pub mod subagent_executor;
pub mod tasks;
pub mod team;
pub mod update;

/// Re-export from archon-tools so downstream crates can use `archon_core::task_manager`.
pub use archon_tools::task_manager;

pub use tasks::{Task, TaskError, TaskEvent, TaskId, TaskState};
