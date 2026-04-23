//! TASK-TUI-628 sandbox module (Gate 1 skeleton).
//!
//! Logical sandbox for tool execution when `PermissionMode::Bubble` is
//! active. Not an OS-level sandbox (no seccomp / chroot / container) —
//! purely an advisory check enforced by the tool dispatcher.
//!
//! # Scope
//!
//!   - `SandboxConfig` — enabled flag + allowlists for read/write paths
//!     + network/shell toggles.
//!   - `SandboxGuard` — wraps a tool execution; delegates to
//!     `check_permission`.
//!   - `check_permission(tool, args, config) -> Result<(), SandboxError>` —
//!     the pure-logic permission check (easily unit-testable).
//!   - `SandboxError` — typed errors: ReadDenied, WriteDenied,
//!     NetworkDenied, ShellDenied.
//!
//! # Reconciliation with TASK-TUI-628.md spec
//!
//! Spec references `crates/archon-tui/src/sandbox/mod.rs` — this file
//! IS at that path, so no reconciliation needed for module location.
//!
//! Spec says `Bubble` variant "between Ask and Yolo" in `PermissionMode`.
//! The actual `archon-permissions::mode::PermissionMode` does NOT have
//! `Ask` or `Yolo` variants — it has `Default` (legacy alias "ask") and
//! `BypassPermissions` (legacy alias "yolo"). Gate 2 inserts `Bubble`
//! between `DontAsk` and `BypassPermissions` per orchestrator guidance.
//!
//! # Gate 1 skeleton
//!
//! Gate 2 (coder subagent) will:
//!   - Flesh out `SandboxConfig` fields + constructors.
//!   - Implement `check_permission` with tool-name routing
//!     (Read/Write/Edit/Glob/Grep → read; Bash → shell; WebFetch →
//!     network; etc.).
//!   - Remove `#[ignore]` on the 4 tests and add real assertions.
//!   - Add `Bubble` variant to `archon-permissions::PermissionMode`
//!     between `DontAsk` and `BypassPermissions`.
//!   - Update `archon-permissions/src/checker.rs` `match self.mode`
//!     at line 65 to include a `Bubble` arm that delegates to
//!     `sandbox::check_permission`.
//!   - Update `archon-permissions/src/mode.rs` `next_mode`, `as_str`,
//!     `from_str` to include Bubble.

use std::path::{Path, PathBuf};

/// Sandbox configuration — allowlists for paths, toggles for
/// network/shell. `enabled` turns the whole sandbox on/off.
#[derive(Debug, Clone, Default)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub read_paths: Vec<PathBuf>,
    pub write_paths: Vec<PathBuf>,
    pub allow_network: bool,
    pub allow_shell: bool,
}

/// Typed errors from `check_permission`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    ReadDenied(PathBuf),
    WriteDenied(PathBuf),
    NetworkDenied,
    ShellDenied,
}

/// Guard wrapping a tool dispatch. Gate 2 will flesh out.
pub struct SandboxGuard<'a> {
    pub config: &'a SandboxConfig,
}

/// Gate 2 implementation: check whether the named tool + args are
/// permitted under the given config. Returns `Ok(())` on permitted,
/// `Err(SandboxError::...)` on denied.
///
/// Routing by tool name:
///   - Read/Glob/Grep → path-arg inspected against `read_paths`.
///   - Write/Edit/NotebookEdit → path-arg inspected against `write_paths`.
///   - Bash/Shell → rejected unless `allow_shell`.
///   - WebFetch/WebSearch → rejected unless `allow_network`.
///   - Unknown tools → default-deny (conservative fallback) when the
///     sandbox is enabled.
///
/// When `config.enabled == false`, the sandbox is transparent: every
/// call returns `Ok(())` with no further inspection.
pub fn check_permission(
    tool: &str,
    args: &[String],
    config: &SandboxConfig,
) -> Result<(), SandboxError> {
    if !config.enabled {
        return Ok(()); // sandbox off = transparent pass-through
    }

    match tool {
        // Read family — inspect first arg as path.
        "Read" | "Glob" | "Grep" => {
            if let Some(arg) = args.first() {
                let p = Path::new(arg);
                if !path_in_any(p, &config.read_paths) {
                    return Err(SandboxError::ReadDenied(p.to_path_buf()));
                }
            }
            Ok(())
        }
        // Write family — inspect first arg as path against write_paths.
        "Write" | "Edit" | "NotebookEdit" => {
            if let Some(arg) = args.first() {
                let p = Path::new(arg);
                if !path_in_any(p, &config.write_paths) {
                    return Err(SandboxError::WriteDenied(p.to_path_buf()));
                }
            }
            Ok(())
        }
        // Shell / command execution.
        "Bash" | "Shell" => {
            if !config.allow_shell {
                return Err(SandboxError::ShellDenied);
            }
            Ok(())
        }
        // Network.
        "WebFetch" | "WebSearch" => {
            if !config.allow_network {
                return Err(SandboxError::NetworkDenied);
            }
            Ok(())
        }
        // Unknown tool: default-deny when sandbox is enabled.
        _ => Err(SandboxError::ShellDenied), // conservative fallback
    }
}

/// Helper: returns `true` if `path` is inside any of `allowed`.
fn path_in_any(path: &Path, allowed: &[PathBuf]) -> bool {
    allowed.iter().any(|base| path.starts_with(base))
}

#[cfg(test)]
mod tests {
    //! Gate 2 sandbox-module unit tests.

    use super::*;

    #[test]
    fn read_allowed_in_sandbox() {
        let config = SandboxConfig {
            enabled: true,
            read_paths: vec![PathBuf::from("/tmp")],
            ..Default::default()
        };
        let result = check_permission("Read", &[String::from("/tmp/file.txt")], &config);
        assert!(
            result.is_ok(),
            "read in allowed path should succeed; got: {:?}",
            result
        );
    }

    #[test]
    fn write_denied_in_readonly_sandbox() {
        let config = SandboxConfig {
            enabled: true,
            read_paths: vec![PathBuf::from("/tmp")],
            write_paths: vec![],
            ..Default::default()
        };
        let result = check_permission("Write", &[String::from("/tmp/file.txt")], &config);
        match result {
            Err(SandboxError::WriteDenied(_)) => {}
            other => panic!("expected WriteDenied, got {:?}", other),
        }
    }

    #[test]
    fn network_denied_when_disallowed() {
        let config = SandboxConfig {
            enabled: true,
            allow_network: false,
            ..Default::default()
        };
        let result = check_permission(
            "WebFetch",
            &[String::from("https://example.com")],
            &config,
        );
        assert_eq!(result, Err(SandboxError::NetworkDenied));
    }

    #[test]
    fn shell_denied_in_bubble_mode() {
        let config = SandboxConfig {
            enabled: true,
            allow_shell: false,
            ..Default::default()
        };
        let result = check_permission("Bash", &[String::from("ls")], &config);
        assert_eq!(result, Err(SandboxError::ShellDenied));
    }
}
