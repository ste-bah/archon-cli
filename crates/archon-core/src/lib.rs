pub mod agent;
pub mod claudemd;
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
pub mod hooks;
pub mod input_format;
pub mod logging;
pub mod output_format;
pub mod plan_explore;
pub mod plan_v2;
pub mod print_mode;
pub mod reasoning;
pub mod schema_validation;
pub mod skills;
pub mod subagent;

/// Re-export from archon-tools so downstream crates can use `archon_core::task_manager`.
pub use archon_tools::task_manager;
