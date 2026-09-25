//! Issue-69: the empty-patch gate against a delivered project artifact.
//!
//! A branch whose deliverables are project artifacts (files under the project
//! data root, outside the repository) legitimately produces an empty git
//! patch. The host digests the declared artifacts before and after the agent
//! runs; a changed digest is work done, exactly as an ignored deliverable
//! carried in the sidecar is.

use super::PatchError;
use super::tests::{isolate, manual_capture};
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::patch_manifest::validate_patch;

const ARTIFACT: &str = ".archon/trading-lab/data/coverage/latest.json";

/// The live shape: no `idempotent_noop`, target hashes that differ, nothing
/// ignored — only the artifact receipt distinguishes it from an empty patch.
fn empty_patch_with_differing_targets() -> super::CapturedPatch {
    let mut captured = manual_capture(b"", &[]);
    captured
        .pre_hashes
        .insert("src/lib.rs".into(), "867593d9".into());
    captured
        .post_hashes
        .insert("src/lib.rs".into(), "dc479f89".into());
    captured
}

#[test]
fn empty_patch_with_delivered_project_artifact_is_work_done() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    let mut captured = empty_patch_with_differing_targets();
    captured.delivered_artifacts = vec![ARTIFACT.to_string()];
    validate_patch(
        &captured,
        &plan,
        &WriteCoordinatorConfig::default(),
        "regenerated the coverage artifact",
    )
    .expect("a changed declared project artifact is a delivery, not an empty patch");
}

#[test]
fn empty_patch_without_delivered_artifact_is_still_empty() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    let captured = empty_patch_with_differing_targets();
    assert!(captured.delivered_artifacts.is_empty());
    match validate_patch(
        &captured,
        &plan,
        &WriteCoordinatorConfig::default(),
        "regenerated the coverage artifact",
    ) {
        Err(PatchError::EmptyPatch) => {}
        other => panic!("expected EmptyPatch without a delivered artifact, got {other:?}"),
    }
}

#[test]
fn delivered_artifact_does_not_excuse_an_unusable_output() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    let mut captured = empty_patch_with_differing_targets();
    captured.delivered_artifacts = vec![ARTIFACT.to_string()];
    let blocked = "**Status:** blocked\nMissing required evidence.";
    match validate_patch(
        &captured,
        &plan,
        &WriteCoordinatorConfig::default(),
        blocked,
    ) {
        Err(PatchError::OutputNotUsable { .. }) => {}
        Ok(_) => panic!("a blocked envelope must still be refused with an artifact receipt"),
        Err(other) => panic!("expected OutputNotUsable, got {other:?}"),
    }
}
