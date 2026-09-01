//! Canonical input manifest for set-level gates.

use archon_workflow::task_set_contract::content_digest;
use archon_workflow::{WorkflowError, WorkflowResult};

use super::workflow_host_command_catalog::HostCommandResolutionContext;

/// Every set-level gate reads the whole task set and the PRD.
///
/// These capabilities pass paths rather than content and deliver no stdin, so
/// without a content term their call identity is identical no matter what the
/// task set says. A gate accepted before a body changed then stays reusable
/// afterwards, and lint and trace never examine the edited bodies.
///
/// Folded into the identity token map, so a changed input yields a different
/// call id and the gate re-executes instead of reusing a stale envelope.
pub(crate) fn set_gate_input_manifest_digest(
    context: &HostCommandResolutionContext,
) -> WorkflowResult<String> {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"set-gate-input-manifest-v1\0");
    canonical.extend_from_slice(context.prd_digest.as_bytes());
    canonical.push(0);

    let mut entries: Vec<(String, String)> = Vec::new();
    let dir = std::fs::read_dir(&context.task_root).map_err(|source| WorkflowError::Io {
        path: context.task_root.clone(),
        source,
    })?;
    for entry in dir {
        let entry = entry.map_err(|source| WorkflowError::Io {
            path: context.task_root.clone(),
            source,
        })?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        // Directories and symlinks are not gate inputs; a symlinked input would
        // also let the digest follow a path outside the task root.
        if !metadata.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt(format!(
                    "task root entry {} has no UTF-8 file name",
                    path.display()
                ))
            })?
            .to_string();
        let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        entries.push((name, content_digest(&bytes)));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, digest) in entries {
        canonical.extend_from_slice(name.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(digest.as_bytes());
        canonical.push(0);
    }
    Ok(content_digest(&canonical))
}
