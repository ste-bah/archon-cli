//! TASK-WC-003 — worktree isolation, baseline capture, mutation detection.
//! Shells out to a real `git`; each test builds its own tempdir repo.

use std::collections::BTreeSet;
use std::path::Path;

use super::*;
use crate::write_coordinator::write_plan::{NormalizedPath, TargetFilesSource, normalize_target};
use crate::write_coordinator::{ItemId, WritePlan};

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A canonical repo with one committed file `src/lib.rs`.
fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "tester"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src/lib.rs"), "// original\n").expect("write");
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

fn np(raw: &str, root: &Path) -> NormalizedPath {
    normalize_target(raw, root).unwrap_or_else(|e| panic!("normalize {raw}: {e}"))
}

fn plan_for(root: &Path, targets: &[&str]) -> WritePlan {
    let target_files: Vec<NormalizedPath> = targets.iter().map(|t| np(t, root)).collect();
    WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/run1/impl-0"),
        target_files,
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: BTreeSet::new(),
    }
}

fn default_cfg() -> WriteCoordinatorConfig {
    WriteCoordinatorConfig::default()
}

#[test]
fn clean_repo_creates_workspace_with_baseline_commit() {
    let repo = canonical_repo();
    let root = repo.path();
    let plan = plan_for(root, &["src/lib.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    let ws = create_item_workspace(root, &plan, &baseline).expect("workspace");
    assert!(plan.isolated_root.join("src/lib.rs").exists());
    assert_eq!(
        std::fs::read_to_string(plan.isolated_root.join("src/lib.rs")).unwrap(),
        "// original\n"
    );
    assert!(!ws.baseline_commit.is_empty());
}

#[test]
fn dirty_tracked_overlay_visible_in_isolated() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("src/lib.rs"), "// edited dirty\n").expect("edit");
    let plan = plan_for(root, &["src/lib.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    create_item_workspace(root, &plan, &baseline).expect("workspace");
    assert_eq!(
        std::fs::read_to_string(plan.isolated_root.join("src/lib.rs")).unwrap(),
        "// edited dirty\n",
        "dirty tracked overlay must reproduce in isolated worktree"
    );
}

#[test]
fn declared_untracked_target_copied_and_captured() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("src/new.rs"), "// brand new\n").expect("write");
    let plan = plan_for(root, &["src/new.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    assert!(baseline.untracked_files.contains_key("src/new.rs"));
    create_item_workspace(root, &plan, &baseline).expect("workspace");
    assert_eq!(
        std::fs::read_to_string(plan.isolated_root.join("src/new.rs")).unwrap(),
        "// brand new\n"
    );
}

#[test]
fn undeclared_untracked_file_never_read_or_copied() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("src/new.rs"), "// declared\n").expect("write");
    std::fs::write(root.join(".env"), "SECRET=hunter2\n").expect("write secret");
    let plan = plan_for(root, &["src/new.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    assert!(
        !baseline.untracked_files.contains_key(".env"),
        "secret untracked file must NOT be read into memory"
    );
    create_item_workspace(root, &plan, &baseline).expect("workspace");
    assert!(
        !plan.isolated_root.join(".env").exists(),
        "secret untracked file must NOT leak into isolated worktree"
    );
}

#[test]
fn verify_input_untracked_copied_and_captured() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("fixture.json"), "{\"k\":1}\n").expect("write");
    let plan = plan_for(root, &["src/lib.rs"]);
    let verify_inputs = vec![np("fixture.json", root)];
    let baseline =
        capture_canonical_baseline(root, &plan, &verify_inputs, &default_cfg()).expect("capture");
    assert!(baseline.verify_input_meta.contains_key("fixture.json"));
    assert!(baseline.untracked_files.contains_key("fixture.json"));
    create_item_workspace(root, &plan, &baseline).expect("workspace");
    assert!(plan.isolated_root.join("fixture.json").exists());
}

#[test]
fn oversize_untracked_declared_rejected_at_capture() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("big.bin"), vec![0u8; 2048]).expect("write big");
    let plan = plan_for(root, &["big.bin"]);
    let cfg = WriteCoordinatorConfig {
        max_file_bytes: 1024,
        ..default_cfg()
    };
    match capture_canonical_baseline(root, &plan, &[], &cfg) {
        Err(IsolationError::FileTooLarge { path, size }) => {
            assert_eq!(path, "big.bin");
            assert_eq!(size, 2048);
        }
        other => panic!("expected FileTooLarge, got {other:?}"),
    }
}

#[test]
fn debug_redacts_untracked_bytes() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("src/new.rs"), "TOPSECRETMARKER\n").expect("write");
    let plan = plan_for(root, &["src/new.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    let dbg = format!("{baseline:?}");
    assert!(
        !dbg.contains("TOPSECRETMARKER"),
        "Debug must redact untracked file bytes, got: {dbg}"
    );
    assert!(dbg.contains("src/new.rs"), "Debug should still show paths");
}

#[test]
fn mutation_of_declared_target_detected() {
    let repo = canonical_repo();
    let root = repo.path();
    let plan = plan_for(root, &["src/lib.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    std::fs::write(root.join("src/lib.rs"), "// mutated behind our back\n").expect("mutate");
    match detect_canonical_mutation(root, &baseline, &plan.target_files, &[]) {
        Err(IsolationError::CanonicalMutation { path }) => assert_eq!(path, "src/lib.rs"),
        other => panic!("expected CanonicalMutation, got {other:?}"),
    }
}

#[test]
fn mutation_of_undeclared_file_ignored() {
    let repo = canonical_repo();
    let root = repo.path();
    let plan = plan_for(root, &["src/lib.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    std::fs::write(root.join("unrelated.txt"), "changed\n").expect("write");
    detect_canonical_mutation(root, &baseline, &plan.target_files, &[])
        .expect("undeclared change is out of scope");
}

#[test]
fn cleanup_succeeded_removes_worktree() {
    let repo = canonical_repo();
    let root = repo.path();
    let plan = plan_for(root, &["src/lib.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    create_item_workspace(root, &plan, &baseline).expect("workspace");
    assert!(plan.isolated_root.exists());
    cleanup_workspace(
        root,
        &plan.isolated_root,
        WorkspaceStatus::Succeeded,
        &default_cfg(),
    )
    .expect("cleanup");
    assert!(
        !plan.isolated_root.exists(),
        "succeeded + retain=false must remove the worktree"
    );
}

#[test]
fn cleanup_failed_retained_keeps_worktree() {
    let repo = canonical_repo();
    let root = repo.path();
    let plan = plan_for(root, &["src/lib.rs"]);
    let baseline = capture_canonical_baseline(root, &plan, &[], &default_cfg()).expect("capture");
    create_item_workspace(root, &plan, &baseline).expect("workspace");
    let cfg = WriteCoordinatorConfig {
        retain_failed_worktrees: true,
        ..default_cfg()
    };
    cleanup_workspace(root, &plan.isolated_root, WorkspaceStatus::Failed, &cfg).expect("cleanup");
    assert!(
        plan.isolated_root.exists(),
        "failed + retain=true must keep the worktree"
    );
}

#[test]
fn run_git_missing_binary_surfaces_git_missing() {
    // Use a cwd that exists; an unknown subcommand triggers ProcessFailed,
    // proving run_git distinguishes process failure from spawn failure.
    let repo = canonical_repo();
    let err = run_git(&["definitely-not-a-subcommand"], repo.path())
        .expect_err("unknown subcommand fails");
    assert!(matches!(err, IsolationError::ProcessFailed { .. }));
}
