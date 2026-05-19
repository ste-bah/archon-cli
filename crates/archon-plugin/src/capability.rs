//! Capability-based security model for TASK-CLI-301.

use std::path::{Component, Path, PathBuf};

// ── PluginCapability ──────────────────────────────────────────────────────────

/// Represents a permission that a plugin may request.
///
/// Plugins are denied-by-default. Each capability must be explicitly granted
/// in the plugin's manifest and accepted by the user during installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCapability {
    /// No capabilities (placeholder / deny-all).
    None,
    /// Read access to specific filesystem paths.
    ReadFs(Vec<PathBuf>),
    /// Write access to specific filesystem paths.
    WriteFs(Vec<PathBuf>),
    /// Outbound network access to specific hostnames.
    Network(Vec<String>),
    /// Explicit high-risk operator approval for unrestricted outbound network.
    NetworkWildcardApproved { approval: String },
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
            if let PluginCapability::ReadFs(allowed) = cap
                && fs_path_allowed(path, allowed)
            {
                return true;
            }
        }
        false
    }

    /// Check if the plugin may write a file at `path`.
    pub fn can_write_fs(&self, path: &Path) -> bool {
        for cap in &self.capabilities {
            if let PluginCapability::WriteFs(allowed) = cap
                && fs_path_allowed(path, allowed)
            {
                return true;
            }
        }
        false
    }

    /// Check if the plugin may make outbound network calls to `hostname`.
    pub fn can_use_network(&self, hostname: &str) -> bool {
        for cap in &self.capabilities {
            match cap {
                PluginCapability::Network(hosts) if hosts.iter().any(|h| h == hostname) => {
                    return true;
                }
                PluginCapability::NetworkWildcardApproved { approval } => {
                    tracing::warn!(
                        approval = %approval,
                        hostname = %hostname,
                        "plugin wildcard network capability used"
                    );
                    return true;
                }
                _ => {}
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

fn fs_path_allowed(path: &Path, allowed: &[PathBuf]) -> bool {
    let Some(requested) = normalize_capability_path(path) else {
        return false;
    };
    allowed.iter().any(|root| {
        normalize_capability_path(root)
            .as_ref()
            .is_some_and(|allowed| requested.starts_with(allowed))
    })
}

fn normalize_capability_path(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Some(canonical);
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let absolute = normalize_lexically(absolute);

    for ancestor in absolute.ancestors() {
        if let Ok(mut canonical_parent) = std::fs::canonicalize(ancestor) {
            let suffix = absolute.strip_prefix(ancestor).ok()?;
            canonical_parent.push(suffix);
            return Some(normalize_lexically(canonical_parent));
        }
    }

    Some(absolute)
}

fn normalize_lexically(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("archon-plugin-{name}-{unique}"))
    }

    #[test]
    fn checker_denies_read_through_symlink_escape() {
        let root = temp_path("read-root");
        let outside = temp_path("read-outside");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("secret.txt"), "nope").expect("secret");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("link")).expect("symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, root.join("link")).expect("symlink");

        let checker = CapabilityChecker::new(vec![PluginCapability::ReadFs(vec![root.clone()])]);
        assert!(!checker.can_read_fs(&root.join("link/secret.txt")));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn checker_denies_write_when_parent_symlink_escapes() {
        let root = temp_path("write-root");
        let outside = temp_path("write-outside");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&outside).expect("outside");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("link")).expect("symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, root.join("link")).expect("symlink");

        let checker = CapabilityChecker::new(vec![PluginCapability::WriteFs(vec![root.clone()])]);
        assert!(!checker.can_write_fs(&root.join("link/new.txt")));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }
}
