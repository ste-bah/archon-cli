//! A branch whose patch did not apply must not report that it landed.

use super::worktree::CompletedWorktreeBranch;
use super::worktree_wave::{WorktreeWaveArtifacts, downgrade_unapplied_branches};
use crate::v2::{WorkflowV2Result, WorkflowV2Status};

fn branch(item_id: &str) -> CompletedWorktreeBranch {
    let mut result = WorkflowV2Result::accepted("wrote the deliverable");
    result.data = serde_json::json!({ "patch_landed": true });
    CompletedWorktreeBranch {
        item_id: item_id.to_string(),
        role: "write".to_string(),
        item_input_hash: None,
        result,
        manifest: None,
        pre_hashes: None,
        workspace_root: std::path::PathBuf::from("/tmp/ws"),
    }
}

fn artifacts(ids: &[&str]) -> WorktreeWaveArtifacts {
    let mut artifacts = WorktreeWaveArtifacts::default();
    for id in ids {
        let completed = branch(id);
        artifacts.results.push(completed.result.clone());
        artifacts.completed.push(completed);
    }
    artifacts
}

/// The defect from run wf-0b0ccf0b: `items_applied` was empty and the item
/// still said `accepted` / `patch_landed: true`, so the authored script -- which
/// reasons per task off the item record, not the batch -- reported the task
/// implemented and moved on to verification.
#[test]
fn a_branch_whose_patch_did_not_apply_is_no_longer_accepted() {
    let mut artifacts = artifacts(&["agents-1-0"]);
    downgrade_unapplied_branches(
        &mut artifacts,
        &[(
            "agents-1-0".into(),
            "StaleBaseline at src/alpha.txt".to_string(),
        )],
    );

    let result = &artifacts.results[0];
    assert_eq!(
        result.status,
        WorkflowV2Status::NeedsReview,
        "a branch that did not land cannot stay accepted"
    );
    assert_eq!(
        result.data.get("patch_landed"),
        Some(&serde_json::Value::Bool(false)),
        "patch_landed must reflect the apply, not the capture"
    );
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id.contains("worktree_patch_unapplied"))
        .expect("a typed gap naming the failure");
    assert!(gap.description.contains("StaleBaseline"), "{gap:?}");
}

/// Only the branches that actually failed are downgraded.
#[test]
fn a_branch_that_applied_cleanly_is_left_alone() {
    let mut artifacts = artifacts(&["agents-1-0", "agents-3-0"]);
    downgrade_unapplied_branches(
        &mut artifacts,
        &[(
            "agents-3-0".into(),
            "StaleBaseline at src/beta.json".to_string(),
        )],
    );

    assert_eq!(artifacts.results[0].status, WorkflowV2Status::Accepted);
    assert_eq!(
        artifacts.results[0].data.get("patch_landed"),
        Some(&serde_json::Value::Bool(true))
    );
    assert_eq!(artifacts.results[1].status, WorkflowV2Status::NeedsReview);
}

/// An apply failure naming an item this wave does not carry changes nothing.
#[test]
fn an_unknown_failed_item_is_ignored_rather_than_mismatched() {
    let mut artifacts = artifacts(&["agents-1-0"]);
    downgrade_unapplied_branches(
        &mut artifacts,
        &[("agents-9-0".into(), "StaleBaseline".to_string())],
    );
    assert_eq!(artifacts.results[0].status, WorkflowV2Status::Accepted);
}

/// The downgrade must be wired into the apply, not merely exist.
///
/// The behavioural tests above call the helper directly, so deleting the call
/// site leaves them green -- verified by sabotage, which passed. A helper that
/// exists and is never invoked is this codebase's signature failure mode and is
/// what produced the defect this file guards, so the call is asserted here.
#[test]
fn the_apply_downgrades_unapplied_branches() {
    let source = include_str!("worktree_wave.rs");
    let apply = source
        .split_once("pub(super) fn apply_worktree_wave(")
        .expect("apply_worktree_wave exists")
        .1;
    let body = apply
        .split_once("\npub(super) fn ")
        .map(|(body, _)| body)
        .unwrap_or(apply);
    assert!(
        body.contains("downgrade_unapplied_branches("),
        "apply_worktree_wave must downgrade branches whose patch did not apply"
    );
    assert!(
        body.contains("items_failed"),
        "the downgrade must be driven by the apply record's failed items"
    );
}
