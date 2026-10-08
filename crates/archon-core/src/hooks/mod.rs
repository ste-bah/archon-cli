//! Hook system — exit-code-based hook execution with condition filtering.
//!
//! # Architecture
//! - `types`    — `HookEvent` (27 variants), `HookConfig`, `HookMatcher`, `HookResult`, `HookError`
//! - `condition`— condition expression evaluator (`"Bash(git *)"` syntax)
//! - `executor` — shell command runner with exit-code semantics
//! - `registry` — `HookRegistry`: loads from .archon/settings.json, fires hooks per event
mod async_diagnostics;
pub mod callback;
pub mod condition;
pub mod context;
mod display;
pub(crate) mod executor;
pub mod function;
pub mod http;
pub mod permissions;
mod registry;
pub mod toml_loader;
mod types;
pub mod watch;

pub use async_diagnostics::{AsyncHookDiagnostic, AsyncHookDiagnosticBatch};
pub use callback::{HookCallback, HookCallbackEntry};
pub use context::{HookContext, HookContextBuilder};
pub use executor::{is_in_hook_agent, set_in_hook_agent};
pub use function::FunctionRegistry;
pub use http::{
    HookHttpTransport, HookHttpTransportBuilder, execute_http_hook, interpolate_env_vars,
    is_localhost,
};
pub use permissions::{PermissionStore, RuntimePermissionStore, apply_permission_updates};
pub use registry::{HookRegistry, HookSummary, compute_hook_id};
pub use toml_loader::{load_hooks_from_toml, parse_hooks_toml};
pub use types::{
    AggregatedHookResult, ElicitationAction, HookCommandType, HookConfig, HookError, HookEvent,
    HookExecutionConfig, HookFailurePolicy, HookMatcher, HookOutcome, HookResult, HookType,
    HooksSettings, PermissionBehavior, PermissionUpdate, PermissionUpdateDestination,
    SourceAuthority,
};
pub use watch::FileWatchManager;
