//! Capability-based security model for TASK-CLI-301.

use std::path::{Path, PathBuf};

// ── PluginCapability ──────────────────────────────────────────────────────────

/// Represents a permission that a plugin may request.
///
/// Plugins are denied-by-default. Each capability must be explicitly granted
/// in the plugin's manifest and accepted by the user during installation.
#[derive(Debug, Clone)]
pub enum PluginCapability {
    /// No capabilities (placeholder / deny-all).
    None,
    /// Read access to specific filesystem paths.
    ReadFs(Vec<PathBuf>),
    /// Write access to specific filesystem paths.
    WriteFs(Vec<PathBuf>),
    /// Outbound network access to specific hostnames.
    Network(Vec<String>),
    /// Permission to register tools with the Archon tool registry.
    ToolRegister,
    /// Permission to register hooks in the Archon hook system.
    HookRegister,
    /// Permission to register slash commands.
    CommandRegister,
    /// Permission to register LSP server configurations.
    LspRegister,
    /// Permission to write to the plugin's own persistent data directory.
    DataDirWrite,
}

// ── CapabilityChecker ─────────────────────────────────────────────────────────

/// Evaluates host function calls against a plugin's declared capabilities.
///
/// All checks are deny-by-default.
#[derive(Debug)]
pub struct CapabilityChecker {
    capabilities: Vec<PluginCapability>,
}

impl CapabilityChecker {
    /// Create a checker from the granted capability list.
    pub fn new(capabilities: Vec<PluginCapability>) -> Self {
        Self { capabilities }
    }

    /// Return the granted capabilities.
    pub fn capabilities(&self) -> &[PluginCapability] {
        &self.capabilities
    }

    /// Check if the plugin may read a file at `path`.
    pub fn can_read_fs(&self, path: &Path) -> bool {
        for cap in &self.capabilities {
            if let PluginCapability::ReadFs(allowed) = cap {
                if allowed.iter().any(|p| path.starts_with(p)) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if the plugin may write a file at `path`.
    pub fn can_write_fs(&self, path: &Path) -> bool {
        for cap in &self.capabilities {
            if let PluginCapability::WriteFs(allowed) = cap {
                if allowed.iter().any(|p| path.starts_with(p)) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if the plugin may make outbound network calls to `hostname`.
    pub fn can_use_network(&self, hostname: &str) -> bool {
        for cap in &self.capabilities {
            if let PluginCapability::Network(hosts) = cap {
                if hosts.iter().any(|h| h == hostname || h == "*") {
                    return true;
                }
            }
        }
        false
    }

    /// Check if the plugin may register tools.
    pub fn can_register_tool(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| matches!(c, PluginCapability::ToolRegister))
    }

    /// Check if the plugin may register hooks.
    pub fn can_register_hook(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| matches!(c, PluginCapability::HookRegister))
    }

    /// Check if the plugin may register commands.
    pub fn can_register_command(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| matches!(c, PluginCapability::CommandRegister))
    }

    /// Check if the plugin may register LSP configurations.
    pub fn can_register_lsp(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| matches!(c, PluginCapability::LspRegister))
    }

    /// Check if the plugin may write to its persistent data directory.
    pub fn can_write_data_dir(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| matches!(c, PluginCapability::DataDirWrite))
    }
}
