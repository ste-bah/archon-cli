//! PluginInstance — wraps a wasmtime Instance for TASK-CLI-301.

use std::path::PathBuf;

// ── PluginInstance ────────────────────────────────────────────────────────────

/// A loaded and initialized WASM plugin instance.
///
/// Holds a reference to the plugin's persistent data directory and the list
/// of tools, hooks, and commands registered during initialization.
#[derive(Debug)]
pub struct PluginInstance {
    /// Plugin identifier.
    pub plugin_name: String,
    /// Persistent data directory for this plugin.
    ///
    /// Created at load time. Path: `~/.local/share/archon/plugins/data/<id>/`.
    /// Injected into the guest as the `ARCHON_PLUGIN_DATA` environment variable.
    pub data_dir: PathBuf,
    /// Tools registered by this plugin during initialization.
    pub registered_tools: Vec<RegisteredTool>,
    /// Hook events registered by this plugin.
    pub registered_hooks: Vec<String>,
    /// Commands registered by this plugin.
    pub registered_commands: Vec<String>,
}

/// A tool registered by a plugin via `archon_register_tool`.
#[derive(Debug, Clone)]
pub struct RegisteredTool {
    /// Tool name as registered (typically `plugin:tool_name`).
    pub name: String,
    /// JSON Schema string for the tool's input parameters.
    pub schema_json: String,
}

impl PluginInstance {
    /// The plugin's identifier.
    pub fn name(&self) -> &str {
        &self.plugin_name
    }

    /// Path to the plugin's persistent data directory.
    ///
    /// This directory is created at load time and persists across plugin updates.
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Tools registered by this plugin.
    pub fn registered_tools(&self) -> &[RegisteredTool] {
        &self.registered_tools
    }

    /// Hook event names registered by this plugin.
    pub fn registered_hooks(&self) -> &[String] {
        &self.registered_hooks
    }

    /// Command names registered by this plugin.
    pub fn registered_commands(&self) -> &[String] {
        &self.registered_commands
    }
}
