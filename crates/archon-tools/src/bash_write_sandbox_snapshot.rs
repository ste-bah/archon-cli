//! Issue-234: the host-side snapshot boundary for the Bash tool's shell on a
//! host with no kernel sandbox (Windows). Captured before the command, restored
//! after it (`bash_write_sandbox::annotate`), so neither a read-only shell nor
//! a write branch can leave a sealed root — the canonical checkout included
//! (Issue-213) — changed.
use std::path::PathBuf;

use archon_shell::write_boundary::{SnapshotBoundary, SnapshotViolation};

use super::{WRITE_BOUNDARY_NOTE_MARKER, WriteBoundary};
use crate::tool::ToolResult;

/// The snapshot for one command: the sealed roots, minus every re-opened
/// writable path (the worktree, the host's temp and cache directories, each
/// declared artifact file) and minus the shared host store. A declared file's
/// missing parent directories are not exempted: the snapshot records files, so
/// creating them is never a change, while another file put in one is -- as a
/// kernel boundary opens only the directory entry, never what goes in it.
pub(super) fn capture(boundary: &WriteBoundary) -> SnapshotBoundary {
    let mut writable = boundary.writable.clone();
    // Exclude the shared host store: the host and other runs write `~/.archon`
    // concurrently, so reverting it could clobber their writes. It stays sealed
    // on kernel hosts.
    writable.extend(user_host_store_dirs());
    SnapshotBoundary::capture(&boundary.protected, &writable)
}

/// The host's own stores under the user's home (`~/.archon/sessions`,
/// `~/.archon/config.toml`). Mirrors
/// `archon_workflow::write_coordinator::sealed_roots::user_host_stores`, which
/// this crate cannot depend on.
fn user_host_store_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()))
        .map(PathBuf::from)
        .filter(|home| home.is_absolute());
    home.map(|home| {
        ["sessions", "config.toml"]
            .iter()
            .map(|store| home.join(".archon").join(store))
            .collect()
    })
    .unwrap_or_default()
}

impl WriteBoundary {
    /// The host-snapshot boundary found the command changed a sealed root. The
    /// changes are already restored; this returns an ordinary tool error
    /// because the command ran and the host detected the change afterwards.
    pub(super) fn snapshot_violation(
        &self,
        result: ToolResult,
        violation: &SnapshotViolation,
    ) -> ToolResult {
        let kind = if self.read_only {
            "This READ-ONLY call's shell"
        } else {
            "This isolated write branch's shell"
        };
        // Keep the boundary guidance note, but do not mark this as a guard
        // refusal: the process ran and the host detected the change afterwards.
        let mut message = format!(
            "Error: Permission denied: {}.\n\n{WRITE_BOUNDARY_NOTE_MARKER} Only your worktree \
             (and the host's temp and cache directories) may be written; everything under {} \
             is sealed, and the host undid the changes. Change the copy in your worktree and \
             report anything outside it in your envelope for the host to land.",
            violation.message(kind),
            self.protected
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !result.content.is_empty() {
            message.push_str("\n\n--- command output ---\n");
            message.push_str(&result.content);
        }
        ToolResult::error(message)
    }
}
