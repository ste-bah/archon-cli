//! PluginHookAdapter — wraps a WASM plugin's hook registration into a HookConfig entry.

use archon_core::hooks::{HookCommandType, HookConfig};

// ── PluginHookAdapter ─────────────────────────────────────────────────────────

/// Adapts a WASM plugin's hook registration to a `HookConfig` entry.
///
/// Hooks are external command invocations matching the CLI-309 command-based model.
/// There is no priority field — execution order follows plugin load order.
pub struct PluginHookAdapter {
    plugin_id: String,
    event: String,
    callback_cmd: String,
}

impl PluginHookAdapter {
    /// Create an adapter for a plugin hook registration.
    pub fn new(plugin_id: String, event: String, callback_cmd: String) -> Self {
        Self {
            plugin_id,
            event,
            callback_cmd,
        }
    }

    /// The hook event name (e.g., `"PreToolUse"`).
    pub fn event(&self) -> &str {
        &self.event
    }

    /// The plugin that registered this hook.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// Convert to a `HookConfig` entry for use with the hook system.
    ///
    /// Hook type is always `Command` (shell command invocation).
    pub fn to_hook_config(&self) -> HookConfig {
        HookConfig {
            hook_type: HookCommandType::Command,
            command: self.callback_cmd.clone(),
            if_condition: None,
            timeout: None,
            once: None,
            r#async: None,
            async_rewake: None,
            status_message: None,
        }
    }
}
