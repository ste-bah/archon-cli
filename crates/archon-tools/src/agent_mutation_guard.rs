use std::fs;
use std::path::PathBuf;

use crate::path_guard::resolve_write_target_path;
use crate::tool::ToolContext;

#[derive(Debug, Clone)]
pub(crate) struct ExpectedMutationSnapshot {
    requested_path: String,
    resolved_path: PathBuf,
    existed: bool,
    hash: Option<String>,
}

pub(crate) fn snapshot_expected_targets(
    requested_paths: &[String],
    cwd: Option<&str>,
    ctx: &ToolContext,
) -> Result<Vec<ExpectedMutationSnapshot>, String> {
    let snapshot_ctx = mutation_context(cwd, ctx);
    requested_paths
        .iter()
        .map(|requested_path| snapshot_one(requested_path, &snapshot_ctx))
        .collect()
}

pub(crate) fn verify_expected_mutations(
    snapshots: &[ExpectedMutationSnapshot],
) -> Result<(), String> {
    let mut unchanged = Vec::new();
    for snapshot in snapshots {
        match fs::read(&snapshot.resolved_path) {
            Ok(bytes) => {
                let new_hash = content_hash(&bytes);
                if snapshot.existed && snapshot.hash.as_deref() == Some(new_hash.as_str()) {
                    unchanged.push(snapshot.requested_path.clone());
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound && !snapshot.existed => {
                unchanged.push(format!("{} (not created)", snapshot.requested_path));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // Deleting a pre-existing expected file still proves mutation.
            }
            Err(err) => {
                return Err(format!(
                    "Failed to verify expected target '{}': {err}",
                    snapshot.resolved_path.display()
                ));
            }
        }
    }
    if unchanged.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "subagent completed but expected target file(s) were unchanged: {}",
            unchanged.join(", ")
        ))
    }
}

fn snapshot_one(
    requested_path: &str,
    ctx: &ToolContext,
) -> Result<ExpectedMutationSnapshot, String> {
    let resolved_path = resolve_write_target_path(requested_path, ctx)?;
    let (existed, hash) = match fs::read(&resolved_path) {
        Ok(bytes) => (true, Some(content_hash(&bytes))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (false, None),
        Err(err) => {
            return Err(format!(
                "Failed to snapshot expected target '{}': {err}",
                resolved_path.display()
            ));
        }
    };
    Ok(ExpectedMutationSnapshot {
        requested_path: requested_path.to_string(),
        resolved_path,
        existed,
        hash,
    })
}

fn mutation_context(cwd: Option<&str>, ctx: &ToolContext) -> ToolContext {
    let mut snapshot_ctx = ctx.clone();
    if let Some(cwd) = cwd.filter(|cwd| !cwd.trim().is_empty()) {
        let cwd = PathBuf::from(cwd);
        snapshot_ctx.working_dir = if cwd.is_absolute() {
            cwd
        } else {
            ctx.working_dir.join(cwd)
        };
    }
    snapshot_ctx
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
