//! When a remediation write positively recorded that it changed nothing:
//! its replay then needs no tree check (`branch_cache_remediation.rs`).

use super::*;

/// A credited write `call_id` recorded as a worktree write that recorded no
/// change, with no receipt that says otherwise: no manifest, or an
/// idempotent (empty) one. Any other manifest names a patch or an ignored
/// deliverable, which contradicts the record, so the ordinary rules judge
/// it. The write mode is the RECORDED call's: a sibling or an earlier
/// session may have written under another mode than the call asking now.
pub(super) fn stands_unchanged(
    v2_store: &WorkflowV2ResultStore,
    call_id: &str,
    outcome: &WorkflowV2BranchOutcome,
    manifest: Option<&PatchManifest>,
) -> bool {
    let worktree = v2_store
        .load_call_record(call_id)
        .ok()
        .flatten()
        .is_some_and(|record| record.call.write_mode == Some(crate::WorkflowV2WriteMode::Worktree));
    worktree
        && matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        )
        && recorded_no_change(outcome)
        && manifest.is_none_or(|manifest| manifest.status == ManifestStatus::IdempotentNoop)
}

/// Whether a worktree write's record positively says it changed nothing.
/// Both markers are the host's, stamped over the agent's data after its
/// answer (`write::worktree_branch_run`): `patch_landed` false (nothing
/// landed against the baseline, ignored deliverables included) and a
/// delivery receipt of no repository change and no changed project artifact.
/// A patch that failed to apply is downgraded with `patch_landed` false but
/// keeps its receipt of a change, so it never qualifies. Serial and
/// coordinated writes carry neither marker, so their agent could write
/// both: never proof (the caller holds the recorded mode to worktree).
fn recorded_no_change(outcome: &WorkflowV2BranchOutcome) -> bool {
    let Some(data) = outcome.result.as_ref().map(|result| &result.data) else {
        return false;
    };
    let delivery = &data["delivery"];
    let unchanged_artifacts = match delivery["kind"].as_str() {
        Some("no_repository_change") => true,
        Some("project_artifact") => delivery["changed_artifact_paths"]
            .as_array()
            .is_some_and(Vec::is_empty),
        _ => false,
    };
    data["patch_landed"].as_bool() == Some(false)
        && delivery["repository_changed"].as_bool() == Some(false)
        && unchanged_artifacts
}
