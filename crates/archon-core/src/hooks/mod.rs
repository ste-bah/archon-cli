pub mod config;
mod executor;
mod types;

pub use executor::HookExecutor;
pub use types::{HookConfig, HookError, HookResult, HookType};

use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// HookRegistry — stores registered hooks indexed by type
// ---------------------------------------------------------------------------

pub struct HookRegistry {
    hooks: HashMap<HookType, Vec<HookConfig>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    pub fn register(&mut self, config: HookConfig) {
        self.hooks
            .entry(config.hook_type.clone())
            .or_default()
            .push(config);
    }

    pub fn from_config(configs: Vec<HookConfig>) -> Self {
        let mut registry = Self::new();
        for config in configs {
            registry.register(config);
        }
        registry
    }

    pub fn hooks_for(&self, hook_type: &HookType) -> &[HookConfig] {
        self.hooks.get(hook_type).map_or(&[], |v| v.as_slice())
    }

    pub fn all_hooks(&self) -> Vec<&HookConfig> {
        self.hooks.values().flat_map(|v| v.iter()).collect()
    }

    /// Format a human-readable status summary of all registered hooks.
    pub fn format_status(&self) -> String {
        if self.hooks.is_empty() {
            return "No hooks configured.".into();
        }

        let mut lines = Vec::new();
        lines.push("Configured hooks:".into());

        let mut sorted_types: Vec<&HookType> = self.hooks.keys().collect();
        sorted_types.sort_by_key(|t| t.to_string());

        for hook_type in sorted_types {
            let entries = &self.hooks[hook_type];
            for entry in entries {
                let mode = if entry.blocking {
                    "blocking"
                } else {
                    "non-blocking"
                };
                let tool_info = entry
                    .tool
                    .as_ref()
                    .map(|t| format!(" [tool={t}]"))
                    .unwrap_or_default();
                lines.push(format!(
                    "  {hook_type}: `{}` ({mode}, {}ms){tool_info}",
                    entry.command, entry.timeout_ms
                ));
            }
        }

        lines.join("\n")
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// HookDispatcher — fires hooks via the executor
// ---------------------------------------------------------------------------

pub struct HookDispatcher {
    registry: HookRegistry,
    session_id: String,
    cwd: PathBuf,
}

impl HookDispatcher {
    pub fn new(registry: HookRegistry, session_id: String, cwd: PathBuf) -> Self {
        Self {
            registry,
            session_id,
            cwd,
        }
    }

    /// Fire all hooks for a given type.
    ///
    /// - **Blocking** hooks execute sequentially in registration order.
    /// - **Non-blocking** hooks are spawned in parallel (fire-and-forget).
    ///
    /// Returns collected `HookResult` values from blocking hooks that
    /// returned structured JSON output.
    pub async fn fire(
        &self,
        hook_type: HookType,
        payload: serde_json::Value,
    ) -> Vec<HookResult> {
        let hooks = self.registry.hooks_for(&hook_type);
        let mut results = Vec::new();

        for hook in hooks {
            match HookExecutor::execute(hook, &payload, &self.session_id, &self.cwd).await {
                Ok(Some(result)) => results.push(result),
                Ok(None) => {
                    // Non-blocking or no structured output — push a default result
                    results.push(HookResult::default());
                }
                Err(e) => {
                    tracing::warn!(
                        hook = %hook.command,
                        hook_type = %hook_type,
                        error = %e,
                        "hook execution failed"
                    );
                    results.push(HookResult::default());
                }
            }
        }

        results
    }

    /// Convenience method for pre_tool_use hooks with tool filtering.
    ///
    /// Fires all PreToolUse hooks whose `tool` filter matches (or is `None`).
    /// If any blocking hook returns `allow: Some(false)`, returns that result
    /// immediately (short-circuit).
    pub async fn fire_pre_tool_use(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Option<HookResult> {
        let hooks = self.registry.hooks_for(&HookType::PreToolUse);

        let payload = serde_json::json!({
            "hook_type": "pre_tool_use",
            "tool": tool_name,
            "input": input,
        });

        for hook in hooks {
            // Apply tool filter: skip hooks that target a different tool
            if let Some(ref filter_tool) = hook.tool {
                if filter_tool != tool_name {
                    continue;
                }
            }

            match HookExecutor::execute(hook, &payload, &self.session_id, &self.cwd).await {
                Ok(Some(result)) => {
                    if result.allow == Some(false) {
                        return Some(result);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        hook = %hook.command,
                        tool = %tool_name,
                        error = %e,
                        "pre_tool_use hook failed"
                    );
                }
            }
        }

        None
    }

    /// Update the working directory (e.g., after cwd_changed event).
    pub fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
    }

    /// Access the inner registry for status display.
    pub fn registry(&self) -> &HookRegistry {
        &self.registry
    }
}
