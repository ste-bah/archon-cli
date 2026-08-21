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
        target_dir_scopes: Vec::new(),
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
        ignored_files: vec![],
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
        Err(PatchError::FileTooManyLines {
            path,
            lines,
            baseline,
            max,
            module_dir,
        }) => {
            assert_eq!(path, "src/lib.rs");
            assert_eq!(lines, 601);
            // The file is 600; only the patch would make it 601. Stating both
            // is what stops the rejection reading as a fact about the file.
            assert_eq!(baseline, 600);
            assert_eq!(max, 500);
            assert_eq!(module_dir, "src/lib/");
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
        Err(PatchError::FileTooManyLines {
            path,
            lines,
            baseline,
            max,
            module_dir,
        }) => {
            assert_eq!(path, "src/new.rs");
            assert_eq!(lines, 501);
            // A new file has no baseline: 0 rather than a misleading number.
            assert_eq!(baseline, 0);
            assert_eq!(max, 500);
            assert_eq!(module_dir, "src/new/");
        }
        other => panic!("expected FileTooManyLines, got {other:?}"),
    }
}

/// The rendered message must state BOTH counts and name a destination.
///
/// Stating only the post-patch size read as a fact about the file: a 483-line
/// file was repeatedly described as 690, which misled two humans with full
/// repo access three times in one day and cost two agent remediation rounds.
/// And "split this file" is a refactor an agent cannot land inside one round,
/// whereas "put it under src/lib/" is a single action.
#[test]
fn the_rejection_states_both_sizes_and_where_new_code_goes() {
    let error = PatchError::FileTooManyLines {
        path: "src/lib.rs".to_string(),
        lines: 690,
        baseline: 483,
        max: 500,
        module_dir: "src/lib/".to_string(),
    };
    let rendered = error.to_string();
    assert!(rendered.contains("would make"), "{rendered}");
    assert!(rendered.contains("690"), "{rendered}");
    assert!(rendered.contains("currently 483"), "{rendered}");
    assert!(rendered.contains("cap 500"), "{rendered}");
    assert!(rendered.contains("src/lib/"), "{rendered}");
}
