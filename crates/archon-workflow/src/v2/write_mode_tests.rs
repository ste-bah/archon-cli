use super::*;
use crate::{WorkflowV2FileRecord, WorkflowV2Result};

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
