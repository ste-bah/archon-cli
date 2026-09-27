//! The host's own bookkeeping basenames (Issue-76), read by both the landing
//! (`archon_workflow::v2::write::host_internal_artifacts`, which drops a
//! changed path with one of these names before any gate reads it) and the
//! tool guard (`archon_tools::workflow_read_guard_targets`, which refuses the
//! write up front). Below both for the same reason as the overlap table: one
//! list, so the guard can never admit what the landing silently drops.

/// Basenames the host writes for its own coordination. A file with one of
/// these names is never a deliverable, wherever it appears in a worktree.
pub const HOST_INTERNAL_ARTIFACT_NAMES: &[&str] = &[
    // `write_coordinator::patch_manifest` — persisted per branch under the run
    // directory, and named by the schema the agent-facing prose already uses.
    "patch_manifest.json",
    // The gate envelope the host replays to a remediating agent.
    "gate-envelope.json",
];

/// Whether `path` names one of the host's own bookkeeping files — by basename,
/// so a copy anywhere in a worktree is caught.
pub fn is_host_internal_artifact_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    HOST_INTERNAL_ARTIFACT_NAMES.contains(&name) || is_read_set_file_name(name)
}

/// The read-set journal naming (`write_read_set::path`): the call id's SHA-256
/// in lowercase hex, `.jsonl`. Matched by shape because the id varies per call.
fn is_read_set_file_name(name: &str) -> bool {
    name.strip_suffix(".jsonl")
        .is_some_and(|stem| stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()))
}
