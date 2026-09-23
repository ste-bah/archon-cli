//! A branch is never failed for an artifact it genuinely produced.
//!
//! The live shape these pin: a write branch runs in an isolated working tree,
//! generates a project deliverable THERE, declares it, and is validated
//! against the destination project root before its output has been applied
//! anywhere. See [`super::branch_produced_artifact`] for the incident.
//!
//! The honest negative is pinned beside it in every case: this check exists
//! because agents claimed outputs they had not written, and an artifact
//! produced in neither tree must still be reported absent.

use super::{WorkflowV2ProjectArtifactContext, normalize_project_artifact_files};
use crate::{WorkflowV2Artifact, WorkflowV2FileRecord, WorkflowV2Result, WorkflowV2Status};

/// A project-data deliverable, in the namespaced shape
/// `namespaced_project_data_artifact` admits.
const DELIVERABLE: &str = ".archon/example-lab/data/reports/history/2026-01-01T00_00_00.json";

struct Trees {
    _temp: tempfile::TempDir,
    project_root: std::path::PathBuf,
    branch_root: std::path::PathBuf,
}

/// A destination project root and a separate branch working tree, neither
/// holding the deliverable yet.
fn trees() -> Trees {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let project_root = temp.path().join("destination-project");
    let branch_root = temp.path().join("branch-worktree");
    std::fs::create_dir_all(&project_root).expect("project root");
    std::fs::create_dir_all(&branch_root).expect("branch root");
    Trees {
        _temp: temp,
        project_root,
        branch_root,
    }
}

fn write_deliverable(root: &std::path::Path) {
    let path = root.join(DELIVERABLE);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("deliverable dir");
    std::fs::write(&path, b"{\"records\":[1]}").expect("deliverable");
}

fn context(trees: &Trees, branch_root: bool) -> WorkflowV2ProjectArtifactContext {
    WorkflowV2ProjectArtifactContext {
        project_root: Some(trees.project_root.display().to_string()),
        repository_root: branch_root.then(|| trees.branch_root.display().to_string()),
        ..Default::default()
    }
}

fn declaring_result() -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "branch result".to_string(),
        artifacts: vec![WorkflowV2Artifact {
            id: "history-entry".to_string(),
            path: DELIVERABLE.to_string(),
            description: Some("timestamped snapshot".to_string()),
        }],
        ..Default::default()
    }
}

fn missing_gaps(result: &WorkflowV2Result) -> Vec<String> {
    result
        .residual_gaps
        .iter()
        .filter(|gap| gap.id.starts_with("missing_project_artifact_"))
        .map(|gap| gap.id.clone())
        .collect()
}

/// The live shape. The branch wrote it in its own working tree, the
/// destination does not have it yet, and the branch must not be failed for
/// the report.
#[test]
fn an_artifact_the_branch_produced_in_its_own_tree_is_not_absent() {
    let trees = trees();
    write_deliverable(&trees.branch_root);
    assert!(
        !trees.project_root.join(DELIVERABLE).exists(),
        "the destination must not hold it, or the test proves nothing"
    );
    let mut result = declaring_result();

    let absent =
        normalize_project_artifact_files("item-1", &mut result, &context(&trees, true)).unwrap();

    assert!(
        absent.is_empty(),
        "a produced artifact must not be reported absent: {absent:?}"
    );
    assert!(
        missing_gaps(&result).is_empty(),
        "no blocking gap for work that exists: {:?}",
        result.residual_gaps
    );
    assert_eq!(
        result.artifacts.len(),
        1,
        "the artifact stays as evidence: {:?}",
        result.artifacts
    );
    assert_eq!(
        result.status,
        WorkflowV2Status::Accepted,
        "the branch keeps the verdict it earned"
    );
}

/// The protection this check exists for. Produced in neither tree, so the
/// claim is still refused — with the same wording and the same blocking gap.
#[test]
fn an_artifact_produced_nowhere_is_still_reported_absent() {
    let trees = trees();
    let mut result = declaring_result();

    let absent =
        normalize_project_artifact_files("item-1", &mut result, &context(&trees, true)).unwrap();

    assert_eq!(
        absent,
        vec![format!("{DELIVERABLE}: it does not exist")],
        "an unwritten claim is still named"
    );
    assert_eq!(
        missing_gaps(&result).len(),
        1,
        "and still raises its blocking gap: {:?}",
        result.residual_gaps
    );
    assert!(
        result.artifacts.is_empty(),
        "an absent path is not artifact evidence"
    );
}

/// Only the branch's OWN tree is consulted. With no second root configured
/// the answer is unchanged, so the fallback cannot be reached by accident.
#[test]
fn without_a_branch_root_the_destination_is_the_only_answer() {
    let trees = trees();
    write_deliverable(&trees.branch_root);
    let mut result = declaring_result();

    let absent =
        normalize_project_artifact_files("item-1", &mut result, &context(&trees, false)).unwrap();

    assert_eq!(
        absent,
        vec![format!("{DELIVERABLE}: it does not exist")],
        "a file in a tree this call was never dispatched into is not evidence"
    );
}

/// An empty file in the branch tree is not evidence there either: the
/// issue-168 rule holds at both locations.
#[test]
fn an_empty_branch_file_is_not_evidence() {
    let trees = trees();
    let path = trees.branch_root.join(DELIVERABLE);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(&path, b"").expect("empty file");
    let mut result = declaring_result();

    let absent =
        normalize_project_artifact_files("item-1", &mut result, &context(&trees, true)).unwrap();

    assert_eq!(
        absent,
        vec![format!("{DELIVERABLE}: it does not exist")],
        "zero bytes in the branch tree satisfies nothing"
    );
}

/// The destination answer still stands on its own when the branch tree has
/// nothing: an artifact already delivered is evidence without the fallback.
#[test]
fn an_artifact_at_the_destination_is_still_evidence() {
    let trees = trees();
    write_deliverable(&trees.project_root);
    let mut result = declaring_result();

    let absent =
        normalize_project_artifact_files("item-1", &mut result, &context(&trees, true)).unwrap();

    assert!(absent.is_empty(), "{absent:?}");
    assert_eq!(result.artifacts.len(), 1);
}

/// One absent path is one claim. An envelope naming a deliverable in both
/// `files_changed` and `artifacts` used to render the identical sentence
/// twice into one error.
#[test]
fn a_path_declared_in_both_lists_is_named_once() {
    let trees = trees();
    let mut result = declaring_result();
    result.files_changed.push(WorkflowV2FileRecord {
        path: DELIVERABLE.to_string(),
        purpose: Some("regenerated deliverable".to_string()),
    });

    let absent =
        normalize_project_artifact_files("item-1", &mut result, &context(&trees, true)).unwrap();

    assert_eq!(
        absent,
        vec![format!("{DELIVERABLE}: it does not exist")],
        "the same claim from two lists folds to one"
    );
}

/// The fold is on the whole rendered claim, not on the path, so two paths
/// each absent for their own reason are both still reported.
#[test]
fn two_distinct_absent_paths_both_survive_the_fold() {
    let trees = trees();
    let second = ".archon/example-lab/data/reports/history/2026-01-02T00_00_00.json";
    let mut result = declaring_result();
    result.files_changed.push(WorkflowV2FileRecord {
        path: second.to_string(),
        purpose: Some("second deliverable".to_string()),
    });

    let absent =
        normalize_project_artifact_files("item-1", &mut result, &context(&trees, true)).unwrap();

    assert_eq!(absent.len(), 2, "{absent:?}");
    assert!(absent.iter().any(|claim| claim.starts_with(DELIVERABLE)));
    assert!(absent.iter().any(|claim| claim.starts_with(second)));
}
