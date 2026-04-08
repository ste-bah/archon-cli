//! Callback/plugin registration types for the hook system.
//!
//! Allows Rust code (plugins, extensions, tests) to register in-process
//! callbacks that fire alongside the regular command/http/prompt hooks.

use std::sync::Arc;

use super::context::HookContext;
use super::types::{HookResult, SourceAuthority};

/// Type alias for hook callbacks.
///
/// The callback receives a borrowed `HookContext` and returns a `HookResult`.
/// Wrapped in `Arc` so it can be cloned into `spawn_blocking` tasks.
pub type HookCallback = Arc<dyn Fn(&HookContext) -> HookResult + Send + Sync>;

/// A registered callback entry, associating a name, authority, timeout, and
/// the actual callback function.
pub struct HookCallbackEntry {
    /// Human-readable name used for logging and for `unregister_callback`.
    pub name: String,
    /// The callback function.
    pub callback: HookCallback,
    /// Source authority tag for merge precedence.
    pub authority: SourceAuthority,
    /// Per-callback timeout in seconds.
    pub timeout_secs: u32,
}
