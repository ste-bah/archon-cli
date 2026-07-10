use super::*;
use crate::{WorkflowV2Artifact, WorkflowV2FileRecord, WorkflowV2Result};

#[test]
fn absolute_changed_file_inside_repository_matches_relative_owned_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let item = WorkflowV2WriteItem::new(
        "impl-TASK-TDL-020",
        WorkflowV2WriteMode::Coordinated,
        vec!["crates/archon-trading/src/data_lake.rs".to_string()],
    );
    let mut result = WorkflowV2Result::accepted("changed owned file");
    result.files_changed.push(WorkflowV2FileRecord::new(
        repo.join("crates/archon-trading/src/data_lake.rs")
            .display()
            .to_string(),
    ));

    validate_changed_files_for_repository(&item, &result, Some(&repo.display().to_string()))
        .expect("absolute in-repo path is owned");
}

#[test]
fn relative_changed_file_matches_absolute_declared_target() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let item = WorkflowV2WriteItem::new(
        "impl-TASK-TDL-020",
        WorkflowV2WriteMode::Coordinated,
        vec![
            repo.join("crates/archon-trading/src/data_lake.rs")
                .display()
                .to_string(),
        ],
    );
    let mut result = WorkflowV2Result::accepted("changed owned file");
    result.files_changed.push(WorkflowV2FileRecord::new(
        "crates/archon-trading/src/data_lake.rs",
    ));

    validate_changed_files_for_repository(&item, &result, Some(&repo.display().to_string()))
        .expect("relative path matches absolute ownership");
}

#[test]
fn absolute_changed_file_outside_repository_remains_unsafe() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo");
    let item = WorkflowV2WriteItem::new(
        "impl-TASK-TDL-020",
        WorkflowV2WriteMode::Coordinated,
        vec!["crates/archon-trading/src/data_lake.rs".to_string()],
    );
    let mut result = WorkflowV2Result::accepted("changed outside file");
    result.files_changed.push(WorkflowV2FileRecord::new(
        temp.path()
            .join("other/crates/archon-trading/src/data_lake.rs")
            .display()
            .to_string(),
    ));

    let error =
        validate_changed_files_for_repository(&item, &result, Some(&repo.display().to_string()))
            .expect_err("outside repo path is unsafe");

    assert!(matches!(
        error,
        WorkflowV2WriteSafetyError::UnsafeTarget { .. }
    ));
}

#[test]
fn instruction_text_target_is_rejected_before_path_resolution() {
    let error = normalize_targets_for_repository(
        "REM-X-001",
        &["Produce or update admissible evidence references for the audit; do not use task docs as repo-owned implementation targets".to_string()],
        None,
    )
    .expect_err("prose target must be unsafe");

    assert!(matches!(
        error,
        WorkflowV2WriteSafetyError::UnsafeTarget { .. }
    ));
}

#[test]
fn target_with_internal_whitespace_is_rejected() {
    let error = normalize_targets_for_repository(
        "REM-X-002",
        &["src/some file.rs".to_string()],
        Some("/repo"),
    )
    .expect_err("whitespace target must be unsafe");

    assert!(matches!(
        error,
        WorkflowV2WriteSafetyError::UnsafeTarget { .. }
    ));
}

#[test]
fn empty_write_ownership_still_requires_explicit_artifact_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let error = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::new(
            "impl-empty",
            WorkflowV2WriteMode::Worktree,
            Vec::new(),
        )])
        .expect_err("plain empty ownership must fail");

    assert!(matches!(
        error,
        WorkflowV2WriteSafetyError::MissingOwnership(_)
    ));
}

#[test]
fn artifact_only_write_item_can_plan_without_repo_targets() {
    let temp = tempfile::tempdir().expect("tempdir");
    let plan = WorkflowV2WritePlanner::new(temp.path())
        .plan(&[WorkflowV2WriteItem::artifact_only(
            "artifact-remediation",
            WorkflowV2WriteMode::Worktree,
        )])
        .expect("artifact-only plan");

    assert_eq!(plan.waves.len(), 1);
    assert!(plan.waves[0].assignments[0].owned_targets.is_empty());
    assert!(plan.waves[0].assignments[0].artifact_only);
}

#[test]
fn artifact_only_accepted_result_requires_artifact_not_repo_edit() {
    let item =
        WorkflowV2WriteItem::artifact_only("artifact-remediation", WorkflowV2WriteMode::Worktree);
    let mut result = WorkflowV2Result::accepted("created project artifact");
    result.artifacts.push(WorkflowV2Artifact {
        id: "gap-audit".to_string(),
        path: ".archon/workflows/wf-test/gap-audit.json".to_string(),
        description: None,
    });

    validate_changed_files_for_repository(&item, &result, Some("/repo"))
        .expect("artifact-only evidence does not need repo ownership");
}

#[test]
fn artifact_only_result_cannot_change_repo_source() {
    let item =
        WorkflowV2WriteItem::artifact_only("artifact-remediation", WorkflowV2WriteMode::Worktree);
    let mut result = WorkflowV2Result::accepted("changed repo source");
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));

    let error = validate_changed_files_for_repository(&item, &result, Some("/repo"))
        .expect_err("artifact-only branch must not edit repo files");

    assert!(matches!(
        error,
        WorkflowV2WriteSafetyError::ChangedFileOutsideOwnership { .. }
    ));
}

#[test]
fn artifact_only_accepted_result_still_requires_artifact_evidence() {
    let item =
        WorkflowV2WriteItem::artifact_only("artifact-remediation", WorkflowV2WriteMode::Worktree);
    let result = WorkflowV2Result::accepted("no artifact reported");

    let error = validate_changed_files_for_repository(&item, &result, Some("/repo"))
        .expect_err("accepted artifact-only branch needs artifact evidence");

    assert!(matches!(
        error,
        WorkflowV2WriteSafetyError::AcceptedWriteWithoutChangedFiles(_)
    ));
}
