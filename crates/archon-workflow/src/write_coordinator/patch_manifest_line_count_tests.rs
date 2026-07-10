use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;
use crate::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
use crate::write_coordinator::{ItemId, WriteCoordinatorConfig, WritePlan};

fn plan_for(root: &Path, target: &str) -> WritePlan {
    WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/run1/impl-0"),
        target_files: vec![normalize_target(target, root).expect("target")],
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: BTreeSet::new(),
    }
}

fn captured(path: &str) -> CapturedPatch {
    CapturedPatch {
        patch_bytes: b"+x\n".to_vec(),
        changed_files: vec![path.to_string()],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::new(),
        baseline_commit: "abc".into(),
    }
}

fn lines(count: usize) -> String {
    (0..count).map(|idx| format!("// line {idx}\n")).collect()
}

#[test]
fn existing_over_limit_file_may_be_touched_without_growth() {
    let root = tempfile::tempdir().expect("root");
    let plan = plan_for(root.path(), "src/lib.rs");
    std::fs::create_dir_all(root.path().join("src")).expect("canonical dir");
    std::fs::create_dir_all(plan.isolated_root.join("src")).expect("isolated dir");
    std::fs::write(root.path().join("src/lib.rs"), lines(600)).expect("baseline");
    std::fs::write(plan.isolated_root.join("src/lib.rs"), lines(600)).expect("isolated");

    validate_patch(
        &captured("src/lib.rs"),
        &plan,
        &WriteCoordinatorConfig::default(),
        "ok",
    )
    .expect("over-limit existing file without growth should pass");
}

#[test]
fn existing_over_limit_file_growth_is_rejected() {
    let root = tempfile::tempdir().expect("root");
    let plan = plan_for(root.path(), "src/lib.rs");
    std::fs::create_dir_all(root.path().join("src")).expect("canonical dir");
    std::fs::create_dir_all(plan.isolated_root.join("src")).expect("isolated dir");
    std::fs::write(root.path().join("src/lib.rs"), lines(600)).expect("baseline");
    std::fs::write(plan.isolated_root.join("src/lib.rs"), lines(601)).expect("isolated");

    match validate_patch(
        &captured("src/lib.rs"),
        &plan,
        &WriteCoordinatorConfig::default(),
        "ok",
    ) {
        Err(PatchError::FileTooManyLines { path, lines, max }) => {
            assert_eq!(path, "src/lib.rs");
            assert_eq!(lines, 601);
            assert_eq!(max, 500);
        }
        other => panic!("expected FileTooManyLines, got {other:?}"),
    }
}

#[test]
fn new_over_limit_file_is_rejected() {
    let root = tempfile::tempdir().expect("root");
    let plan = plan_for(root.path(), "src/new.rs");
    std::fs::create_dir_all(plan.isolated_root.join("src")).expect("isolated dir");
    std::fs::write(plan.isolated_root.join("src/new.rs"), lines(501)).expect("isolated");

    match validate_patch(
        &captured("src/new.rs"),
        &plan,
        &WriteCoordinatorConfig::default(),
        "ok",
    ) {
        Err(PatchError::FileTooManyLines { path, lines, max }) => {
            assert_eq!(path, "src/new.rs");
            assert_eq!(lines, 501);
            assert_eq!(max, 500);
        }
        other => panic!("expected FileTooManyLines, got {other:?}"),
    }
}
