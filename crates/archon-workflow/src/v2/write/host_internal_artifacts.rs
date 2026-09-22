//! Issue-76: the host's own bookkeeping is never a branch deliverable.
//!
//! The host writes coordination files of its own while a branch runs — the
//! per-branch write manifest, the gate envelope, the read-set journal. They
//! live under the run directory, they are written and read by the host alone,
//! and none of them is a work product of the task.
//!
//! Live on `wf-0ddadd81`: a branch result advertised the host's write manifest
//! as one of the branch's ARTIFACTS, the replayed "previous attempt was
//! REJECTED" envelope rendered that entry into the coder's prompt eight times,
//! and the coder — reading it as an artifact it was required to produce —
//! copied the host's file into its worktree at the repository root. Nothing
//! below refused it: a path directly at the repository root is exempt from the
//! scope-roots drop (Issue-27), nothing else in the wave claimed it, so it was
//! granted, declared and committed into the target repository.
//!
//! Two independent layers close that, and this module holds the name list both
//! read so they cannot drift apart:
//!
//! 1. Nothing host-internal is recorded as a branch artifact, so no prompt can
//!    advertise one as a deliverable (see `worktree_branch_b`, where the
//!    manifest artifact used to be pushed).
//! 2. A changed path whose basename is one of these names is never landed:
//!    [`drop_host_internal_changes`] removes or restores it in the worktree
//!    before any gate reads it, capture filters it out of the diff as a
//!    backstop, and the branch is NOT failed for it — it is reported as a
//!    review gap, the way Issue-13 and Issue-27 report their drops.
//!
//! The judgement is on the BASENAME, not on where the file sits: the host's
//! copy lives outside the repository, so only a reproduction of it can ever
//! appear in a worktree, and it can appear anywhere in the tree.
use super::*;

use archon_write_plan::WritePlan;

/// Basenames the host writes for its own coordination. A file with one of
/// these names is never a deliverable, wherever it appears in a worktree.
pub(crate) const HOST_INTERNAL_ARTIFACT_NAMES: &[&str] = &[
    // `write_coordinator::patch_manifest` — persisted per branch under the run
    // directory, and named by the schema the agent-facing prose already uses.
    "patch_manifest.json",
    // The gate envelope the host replays to a remediating agent.
    "gate-envelope.json",
];

/// Gap id prefix for the host-internal paths a branch dropped.
pub(crate) const HOST_INTERNAL_DROPPED_GAP_PREFIX: &str = "host_internal_artifact_dropped_";

/// Whether `path` names one of the host's own bookkeeping files — by basename,
/// so a copy anywhere in a worktree is caught.
pub(crate) fn is_host_internal_artifact_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    HOST_INTERNAL_ARTIFACT_NAMES.contains(&name) || is_read_set_file_name(name)
}

/// The read-set journal naming (`write_read_set::path`): the call id's SHA-256
/// in lowercase hex, `.jsonl`. Matched by shape because the id varies per call.
fn is_read_set_file_name(name: &str) -> bool {
    name.strip_suffix(".jsonl")
        .is_some_and(|stem| stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Drop every host-internal file the branch left in its worktree, and strike
/// the same paths from the envelope, returning the paths actually dropped.
///
/// Runs BEFORE the scope grant resolves and before any ownership gate, for the
/// same reason as the Issue-13 and Issue-27 drops: a path the worktree no
/// longer holds is never a grant candidate, never declared in the manifest and
/// never in the diff, so it cannot land and cannot fail the branch either. The
/// envelope entry goes with it — gate 1 judges what the envelope reports, and
/// a reported path with nothing behind it would be refused as an undeclared
/// write.
pub(super) fn drop_host_internal_changes(
    plan: &WritePlan,
    result: &mut WorkflowV2Result,
) -> Vec<String> {
    result
        .files_changed
        .retain(|file| !is_host_internal_artifact_path(&file.path));
    let changed =
        crate::write_coordinator::patch_manifest::workspace_changed_paths(&plan.isolated_root)
            .unwrap_or_default();
    let candidates: Vec<String> = changed
        .into_iter()
        .filter(|path| is_host_internal_artifact_path(path))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    crate::write_coordinator::whitespace_only::restore_or_remove(&plan.isolated_root, &candidates)
}

/// Record the host-internal paths this branch dropped as a review gap and an
/// evidence line, whatever the branch's status: the files were removed or
/// restored in the worktree before any gate read them, so a reviewer has to be
/// told from here. Status and summary are untouched.
pub(super) fn report_host_internal_drops(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    dropped: &[String],
) {
    if dropped.is_empty() {
        return;
    }
    let paths = dropped.join(", ");
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{HOST_INTERNAL_DROPPED_GAP_PREFIX}{}",
            sanitize_v2_path_segment(branch_id)
        ),
        description: format!(
            "write item '{branch_id}' wrote {} path(s) into its worktree that are the host's \
             own coordination bookkeeping, not work products: {paths}. Each was removed or \
             restored in the worktree and excluded from the patch rather than failing the \
             branch. Never reproduce a host file inside the repository: the host writes and \
             reads its own records, and a copy of one is not a deliverable.",
            dropped.len()
        ),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        format!(
            "host-internal bookkeeping dropped from the patch ({} path(s)): {paths}",
            dropped.len()
        ),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "host_internal_dropped".to_string(),
            serde_json::json!(dropped),
        );
    }
}

#[cfg(test)]
#[path = "host_internal_artifacts_tests.rs"]
mod tests;
