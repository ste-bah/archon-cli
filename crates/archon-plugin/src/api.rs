//! PluginRegistry — central registry for plugin-provided tools, hooks, and commands.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use archon_core::hooks::{HookCommandType, HookConfig};

use crate::abi::VALID_HOOK_EVENTS;
use crate::adapter_tool::PluginToolAdapter;
use crate::capability::PluginCapability;
use crate::host::WasmPluginHost;
use crate::instance::PluginInstance;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PluginRegistryError {
    #[error("invalid JSON schema: {0}")]
    InvalidSchema(String),
    #[error("duplicate tool name: {0}")]
    DuplicateTool(String),
    #[error("duplicate command name: {0}")]
    DuplicateCommand(String),
    #[error("invalid hook event: {0}")]
    InvalidHookEvent(String),
}

// ── Internal entries ──────────────────────────────────────────────────────────

struct ToolEntry {
    plugin_id: String,
    namespaced_name: String,
    // Stored for future schema-aware tool dispatch and capability filtering.
    #[allow(dead_code)]
    schema_json: String,
    #[allow(dead_code)]
    capabilities: Vec<PluginCapability>,
}

struct HookEntry {
    plugin_id: String,
    // Stored for future per-event filtering in hook_entries().
    #[allow(dead_code)]
    event: String,
    hook_config: HookConfig,
}

struct CommandEntry {
    plugin_id: String,
    namespaced_name: String,
}

// ── PluginRegistry ────────────────────────────────────────────────────────────

/// Central registry tracking all plugin-provided tools, hooks, and commands.
pub struct PluginRegistry {
    tools: Vec<ToolEntry>,
    hooks: Vec<HookEntry>,
    commands: Vec<CommandEntry>,
    /// Per-plugin enable/disable state. Absent = enabled.
    enabled: HashMap<String, bool>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            hooks: Vec::new(),
            commands: Vec::new(),
            enabled: HashMap::new(),
        }
    }

    // ── Tool registration ─────────────────────────────────────────────────

    /// Register a tool from a plugin.
    ///
    /// Tool name is namespaced as `plugin_id:tool_name` (colon separator).
    /// Rejects duplicate namespaced names and invalid JSON schemas.
    pub fn register_tool(
        &mut self,
        plugin_id: &str,
        tool_name: &str,
        schema_json: String,
        capabilities: Vec<PluginCapability>,
    ) -> Result<(), PluginRegistryError> {
        // Validate JSON schema
        serde_json::from_str::<serde_json::Value>(&schema_json)
            .map_err(|e| PluginRegistryError::InvalidSchema(e.to_string()))?;

        let namespaced = format!("{plugin_id}:{tool_name}");

        // Reject duplicates
        if self.tools.iter().any(|e| e.namespaced_name == namespaced) {
            return Err(PluginRegistryError::DuplicateTool(namespaced));
        }

        self.tools.push(ToolEntry {
            plugin_id: plugin_id.to_string(),
            namespaced_name: namespaced,
            schema_json,
            capabilities,
        });
        Ok(())
    }

    /// All namespaced tool names registered across all plugins.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools
            .iter()
            .map(|e| e.namespaced_name.as_str())
            .collect()
    }

    /// Number of registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    // ── Hook registration ─────────────────────────────────────────────────

    /// Register a hook from a plugin.
    ///
    /// `event` must be a valid hook event name. `callback_cmd` is the shell
    /// command the hook system will invoke when the event fires.
    pub fn register_hook(
        &mut self,
        plugin_id: &str,
        event: &str,
        callback_cmd: &str,
    ) -> Result<(), PluginRegistryError> {
        if !VALID_HOOK_EVENTS.contains(&event) {
            return Err(PluginRegistryError::InvalidHookEvent(event.to_string()));
        }

        let hook_config = HookConfig {
            hook_type: HookCommandType::Command,
            command: callback_cmd.to_string(),
            if_condition: None,
            timeout: None,
            once: None,
            r#async: None,
            async_rewake: None,
            status_message: None,
        };

        self.hooks.push(HookEntry {
            plugin_id: plugin_id.to_string(),
            event: event.to_string(),
            hook_config,
        });
        Ok(())
    }

    /// All `HookConfig` entries registered by plugins, in registration order.
    pub fn hook_entries(&self) -> Vec<&HookConfig> {
        self.hooks.iter().map(|e| &e.hook_config).collect()
    }

    /// Number of registered hooks.
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    // ── Command registration ──────────────────────────────────────────────

    /// Register a command from a plugin.
    ///
    /// Rejects duplicate namespaced command names (`plugin_id:name`).
    pub fn register_command(
        &mut self,
        plugin_id: &str,
        name: &str,
    ) -> Result<(), PluginRegistryError> {
        let namespaced = format!("{plugin_id}:{name}");
        if self
            .commands
            .iter()
            .any(|e| e.namespaced_name == namespaced)
        {
            return Err(PluginRegistryError::DuplicateCommand(namespaced));
        }
        self.commands.push(CommandEntry {
            plugin_id: plugin_id.to_string(),
            namespaced_name: namespaced,
        });
        Ok(())
    }

    /// Number of registered commands.
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    // ── unregister_all ────────────────────────────────────────────────────

    /// Atomically clear all registrations for a plugin.
    ///
    /// Uses clear-before-register semantics: removes all existing entries for
    /// `plugin_id` first. The caller is then responsible for re-registering
    /// the current set. This prevents stale + new registrations coexisting.
    pub fn unregister_all(&mut self, plugin_id: &str) {
        self.tools.retain(|e| e.plugin_id != plugin_id);
        self.hooks.retain(|e| e.plugin_id != plugin_id);
        self.commands.retain(|e| e.plugin_id != plugin_id);
    }

    // ── Enable / disable ──────────────────────────────────────────────────

    /// Set the enabled state for a plugin.
    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) {
        self.enabled.insert(plugin_id.to_string(), enabled);
    }

    /// Whether a plugin is enabled. Plugins not explicitly set are enabled by default.
    pub fn is_enabled(&self, plugin_id: &str) -> bool {
        *self.enabled.get(plugin_id).unwrap_or(&true)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Source ID ─────────────────────────────────────────────────────────────────

/// Canonical plugin source identifier: `{name}@{marketplace}`.
pub fn plugin_source_id(name: &str, marketplace: &str) -> String {
    format!("{name}@{marketplace}")
}

// ── tools_from_plugin_instance ────────────────────────────────────────────────

/// Create one `Box<dyn Tool>` per tool registered by `instance`.
///
/// Each adapter wraps the live WASM host so calls are dispatched into the guest.
/// Returns an empty vec if the instance registered no tools.
pub fn tools_from_plugin_instance(
    plugin_id: &str,
    instance: &PluginInstance,
    host: Arc<Mutex<WasmPluginHost>>,
) -> Vec<Box<dyn archon_tools::tool::Tool>> {
    instance
        .registered_tools()
        .iter()
        .map(|t| {
            let adapter = PluginToolAdapter::new_with_host(
                plugin_id.to_string(),
                t.name.clone(),
                String::new(),
                t.schema_json.clone(),
                Arc::clone(&host),
            );
            Box::new(adapter) as Box<dyn archon_tools::tool::Tool>
        })
        .collect()
}
