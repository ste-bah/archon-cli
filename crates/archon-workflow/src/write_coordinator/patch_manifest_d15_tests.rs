use std::collections::BTreeSet;
use std::path::Path;

use super::*;
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace,
};
use crate::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
use crate::write_coordinator::{ItemId, WritePlan};

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "t"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(root.join("src/lib.rs"), "// original\n").expect("lib");
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

fn plan(root: &Path, targets: &[&str], scopes: &[&str]) -> WritePlan {
    WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/run1/impl-0"),
        target_files: targets
            .iter()
            .map(|t| normalize_target(t, root).unwrap())
            .collect(),
        target_dir_scopes: scopes
            .iter()
            .map(|t| normalize_target(t, root).unwrap())
            .collect(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: BTreeSet::new(),
    }
}

#[test]
fn absent_declared_target_matches_absent_worktree_target() {
    let repo = repo();
    let plan = plan(repo.path(), &["src/missing.rs"], &[]);
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(repo.path(), &plan, &[], &cfg).expect("baseline");
    let ws = create_item_workspace(repo.path(), &plan, &baseline).expect("workspace");

    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");

    assert_eq!(captured.pre_hashes, captured.post_hashes);
    assert!(!captured.post_hashes.contains_key("src/missing.rs"));
    validate_patch(&captured, &plan, &cfg, "No missing work found.").expect("noop");
}

#[test]
fn directory_scope_allows_child_create_without_hashing_directory() {
    let repo = repo();
    let plan = plan(repo.path(), &["src/lib.rs"], &["src/generated"]);
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(repo.path(), &plan, &[], &cfg).expect("baseline");
    let ws = create_item_workspace(repo.path(), &plan, &baseline).expect("workspace");
    std::fs::create_dir_all(plan.isolated_root.join("src/generated")).expect("scope dir");
    std::fs::write(plan.isolated_root.join("src/generated/new.rs"), "// new\n").expect("child");

    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");

    assert_eq!(captured.created_files, vec!["src/generated/new.rs"]);
    assert!(!captured.post_hashes.contains_key("src/generated"));
    validate_patch(&captured, &plan, &cfg, "ok").expect("scoped child is owned");
}
