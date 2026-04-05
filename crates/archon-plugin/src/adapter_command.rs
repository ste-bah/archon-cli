//! PluginCommandAdapter — wraps a WASM plugin's command registration.

// ── PluginCommandAdapter ──────────────────────────────────────────────────────

/// Adapts a WASM plugin's command registration into the command system.
///
/// Command names are namespaced as `plugin_id:command_name`.
pub struct PluginCommandAdapter {
    plugin_id: String,
    command_name: String,
}

impl PluginCommandAdapter {
    /// Create an adapter for a plugin command registration.
    pub fn new(plugin_id: String, command_name: String) -> Self {
        Self {
            plugin_id,
            command_name,
        }
    }

    /// The plugin that registered this command.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// The namespaced command name: `plugin_id:command_name`.
    pub fn namespaced_name(&self) -> String {
        format!("{}:{}", self.plugin_id, self.command_name)
    }

    /// The local (un-namespaced) command name.
    pub fn command_name(&self) -> &str {
        &self.command_name
    }
}
