//! Hook system — exit-code-based hook execution with condition filtering.
//!
//! # Architecture
//! - `types`    — `HookEvent` (27 variants), `HookConfig`, `HookMatcher`, `HookResult`, `HookError`
//! - `condition`— condition expression evaluator (`"Bash(git *)"` syntax)
//! - `executor` — shell command runner with exit-code semantics
//! - `registry` — `HookRegistry`: loads from settings.json, fires hooks per event
//! - `config`   — legacy TOML config loader (kept for backward compat)

pub mod condition;
pub mod config;
mod executor;
mod registry;
mod types;

pub use registry::HookRegistry;
pub use types::{
    HookCommandType, HookConfig, HookError, HookEvent, HookMatcher, HookResult, HooksSettings,
};

/// Backward-compat alias: `HookType` is an alias for `HookEvent`.
pub use types::HookType;
