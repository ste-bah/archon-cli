//! Function hook executor — in-process named functions for the hook system.
//!
//! Provides a `FunctionRegistry` with built-in functions (`noop`, `block_all`)
//! and the ability to register custom functions at runtime.

use std::collections::HashMap;

use super::context::HookContext;
use super::types::{HookOutcome, HookResult};

/// Signature for a function hook: takes a `&HookContext` and returns a `HookResult`.
pub type HookFn = fn(&HookContext) -> HookResult;

/// Registry of named function hooks.
///
/// Ships with two built-in functions:
/// - `"noop"` — returns `HookResult::default()` (Success, no side effects)
/// - `"block_all"` — returns a Blocking result with a descriptive reason
///
/// Unknown function names are treated as fail-open (warn + Success).
#[derive(Debug)]
pub struct FunctionRegistry {
    functions: HashMap<String, HookFn>,
}

impl FunctionRegistry {
    /// Create a new registry pre-loaded with built-in functions.
    pub fn new() -> Self {
        let mut functions: HashMap<String, HookFn> = HashMap::new();
        functions.insert("noop".to_string(), builtin_noop as HookFn);
        functions.insert("block_all".to_string(), builtin_block_all as HookFn);
        Self { functions }
    }

    /// Register a custom named function.
    pub fn register(&mut self, name: String, func: HookFn) {
        self.functions.insert(name, func);
    }

    /// Execute the named function. Returns `HookResult::default()` (fail-open)
    /// if the function name is not found.
    pub fn execute(&self, name: &str, ctx: &HookContext) -> HookResult {
        match self.functions.get(name) {
            Some(func) => func(ctx),
            None => {
                tracing::warn!(
                    function = %name,
                    "unknown function hook name; returning success (fail-open)"
                );
                HookResult::default()
            }
        }
    }
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in functions
// ---------------------------------------------------------------------------

/// No-op function: always returns Success with no side effects.
fn builtin_noop(_ctx: &HookContext) -> HookResult {
    HookResult::default()
}

/// Block-all function: always returns Blocking with a descriptive reason.
fn builtin_block_all(_ctx: &HookContext) -> HookResult {
    HookResult {
        outcome: HookOutcome::Blocking,
        reason: Some("Blocked by block_all function".to_string()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::HookEvent;

    fn test_context() -> HookContext {
        HookContext::builder(HookEvent::PreToolUse)
            .session_id("test-session".into())
            .cwd("/tmp".into())
            .build()
    }

    #[test]
    fn noop_returns_success() {
        let registry = FunctionRegistry::new();
        let ctx = test_context();
        let result = registry.execute("noop", &ctx);
        assert!(result.is_success());
        assert!(result.reason.is_none());
    }

    #[test]
    fn block_all_returns_blocking() {
        let registry = FunctionRegistry::new();
        let ctx = test_context();
        let result = registry.execute("block_all", &ctx);
        assert!(result.is_blocking());
        assert!(result.reason.as_deref().unwrap().contains("block_all"));
    }

    #[test]
    fn unknown_function_failopen() {
        let registry = FunctionRegistry::new();
        let ctx = test_context();
        let result = registry.execute("does_not_exist", &ctx);
        assert!(result.is_success());
    }

    #[test]
    fn custom_function_registration() {
        let mut registry = FunctionRegistry::new();
        registry.register("custom".to_string(), |_ctx: &HookContext| -> HookResult {
            HookResult {
                outcome: HookOutcome::NonBlockingError,
                reason: Some("custom reason".to_string()),
                ..Default::default()
            }
        });
        let ctx = test_context();
        let result = registry.execute("custom", &ctx);
        assert_eq!(result.outcome, HookOutcome::NonBlockingError);
    }
}
