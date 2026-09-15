//! The whitespace-only drop against a real sealed worktree: what is found,
//! what is restored, and what is deliberately left alone.

use std::path::Path;

use super::whitespace_only::{
    WorktreeChanges, restore_or_remove, restore_to_baseline, undeclared_whitespace_only_changes,
    whitespace_only_change, worktree_changes,
};
use super::worktree_isolation::{capture_canonical_baseline, create_item_workspace};
use super::write_plan::{TargetFilesSource, WritePlan, normalize_target};
use super::{WriteCoordinatorConfig, worktree_isolation::run_git};

fn git(root: &Path, args: &[&str]) {
    run_git(args, root).expect("git");
}

/// A canonical repository with one commit, and a sealed worktree for a plan
/// that declares `owned.txt` only.
fn sealed(dir: &Path) -> WritePlan {
    let canonical = dir.join("canonical");
    std::fs::create_dir_all(canonical.join("src")).unwrap();
    git(&canonical, &["init", "-q"]);
    git(&canonical, &["config", "user.name", "t"]);
    git(&canonical, &["config", "user.email", "t@example.invalid"]);
    std::fs::write(canonical.join("owned.txt"), "baseline\n").unwrap();
    std::fs::write(canonical.join("src/formatted.txt"), "fn f() {\n    1\n}\n").unwrap();
    std::fs::write(canonical.join("src/real.txt"), "fn g() {}\n").unwrap();
    git(&canonical, &["add", "."]);
    git(&canonical, &["commit", "-qm", "baseline"]);
    let plan = WritePlan {
        run_id: "run".into(),
        stage_id: "stage".into(),
        item_id: "item".into(),
        canonical_root: canonical.clone(),
        isolated_root: dir.join("iso"),
        target_files: vec![normalize_target("owned.txt", &canonical).unwrap()],
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: Vec::new(),
        verify_inputs: Vec::new(),
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    };
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(&canonical, &plan, &[], &cfg).unwrap();
    create_item_workspace(&canonical, &plan, &baseline).unwrap();
    plan
}

#[test]
fn comparison_is_bytes_minus_ascii_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let before = dir.path().join("before");
    let after = dir.path().join("after");
    std::fs::write(&before, "fn f() {\n    1\n}\n").unwrap();
    std::fs::write(&after, "fn f() {\n\t1\n}\n\n").unwrap();
    assert!(whitespace_only_change(&before, &after));
    std::fs::write(&after, "fn f() {\n    1,\n}\n").unwrap();
    assert!(
        !whitespace_only_change(&before, &after),
        "a comma is a real change"
    );
    std::fs::write(&after, "fn f() {\n    1\n}\n").unwrap();
    assert!(
        !whitespace_only_change(&before, &after),
        "identical is not a change"
    );
    assert!(!whitespace_only_change(
        &before,
        &dir.path().join("missing")
    ));
    assert!(!whitespace_only_change(&dir.path().join("missing"), &after));
}

/// The live shape: the agent edits its declared file, re-indents one file it
/// does not own, and makes a real edit to another it does not own. Only the
/// re-indented one is found, only it is restored, and the other two are left
/// exactly as the agent wrote them.
#[test]
fn only_undeclared_whitespace_only_paths_are_found_and_restored() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path());
    let iso = &plan.isolated_root;
    std::fs::write(iso.join("owned.txt"), "baseline\n\n").unwrap();
    std::fs::write(iso.join("src/formatted.txt"), "fn f() {\n\t1\n}\n\n").unwrap();
    std::fs::write(iso.join("src/real.txt"), "fn g() { 2 }\n").unwrap();
    std::fs::write(iso.join("src/created.txt"), "\n").unwrap();

    let found = undeclared_whitespace_only_changes(&plan);
    assert_eq!(found, vec!["src/formatted.txt".to_string()]);

    let restored = restore_to_baseline(iso, &found);
    assert_eq!(restored, found);
    assert_eq!(
        std::fs::read_to_string(iso.join("src/formatted.txt")).unwrap(),
        "fn f() {\n    1\n}\n"
    );
    assert_eq!(
        std::fs::read_to_string(iso.join("owned.txt")).unwrap(),
        "baseline\n\n",
        "a declared file is never touched"
    );
    assert_eq!(
        std::fs::read_to_string(iso.join("src/real.txt")).unwrap(),
        "fn g() { 2 }\n",
        "a real change is never touched"
    );
    assert!(iso.join("src/created.txt").exists());
    let changed = super::patch_manifest::workspace_changed_paths(iso).unwrap();
    assert_eq!(
        changed,
        vec![
            "owned.txt".to_string(),
            "src/created.txt".to_string(),
            "src/real.txt".to_string()
        ],
        "capture must no longer see the restored path"
    );
    assert!(undeclared_whitespace_only_changes(&plan).is_empty());
}

#[test]
fn a_path_git_cannot_restore_is_left_alone_and_not_claimed() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path());
    let iso = &plan.isolated_root;
    std::fs::write(iso.join("src/created.txt"), "new\n").unwrap();
    let restored = restore_to_baseline(iso, &["src/created.txt".to_string()]);
    assert!(restored.is_empty());
    assert_eq!(
        std::fs::read_to_string(iso.join("src/created.txt")).unwrap(),
        "new\n"
    );
}

#[test]
fn a_worktree_that_is_not_a_checkout_finds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut plan = sealed(dir.path());
    plan.isolated_root = dir.path().join("plain");
    std::fs::create_dir_all(plan.isolated_root.join("src")).unwrap();
    std::fs::write(
        plan.isolated_root.join("src/formatted.txt"),
        "fn f() {\n\t1\n}\n\n",
    )
    .unwrap();
    assert!(undeclared_whitespace_only_changes(&plan).is_empty());
}

/// The one scan the grant reads (Issue-16): every change versus the sealed
/// baseline, partitioned by the plan. A declared edit, a real undeclared edit,
/// a created file, a deleted file and an untracked file are all found; a
/// re-indented undeclared file is the whitespace partition; an ignored file
/// is not a change at all.
#[test]
fn worktree_changes_partitions_every_change_against_the_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path());
    let iso = &plan.isolated_root;
    std::fs::write(iso.join("owned.txt"), "implemented\n").unwrap();
    std::fs::write(iso.join("src/formatted.txt"), "fn f() {\n\t1\n}\n\n").unwrap();
    std::fs::write(iso.join("src/real.txt"), "fn g() { 2 }\n").unwrap();
    std::fs::write(iso.join("src/created.txt"), "new\n").unwrap();
    std::fs::write(iso.join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(iso.join("ignored.txt"), "scratch\n").unwrap();
    git(iso, &["rm", "-q", "--cached", "src/real.txt"]);
    git(iso, &["add", "src/real.txt"]);
    std::fs::remove_file(iso.join("src/real.txt")).unwrap();
    assert_eq!(
        worktree_changes(&plan),
        WorktreeChanges {
            declared: vec!["owned.txt".to_string()],
            undeclared: vec![
                ".gitignore".to_string(),
                "src/created.txt".to_string(),
                "src/real.txt".to_string(),
            ],
            whitespace_only: vec!["src/formatted.txt".to_string()],
        }
    );
    assert_eq!(
        undeclared_whitespace_only_changes(&plan),
        vec!["src/formatted.txt".to_string()]
    );
}

/// The out-of-scope drop (Issue-27) meets created files, which
/// `restore_to_baseline` cannot touch: a modified file is restored, a deleted
/// one is brought back, a created one — untracked or staged — is removed, and
/// afterwards capture's scan sees none of them. A path with nothing on either
/// side is not claimed as dropped.
#[test]
fn restore_or_remove_handles_modified_deleted_and_created_files() {
    let dir = tempfile::tempdir().unwrap();
    let plan = sealed(dir.path());
    let iso = &plan.isolated_root;
    std::fs::write(iso.join("src/real.txt"), "fn g() { 2 }\n").unwrap();
    std::fs::remove_file(iso.join("src/formatted.txt")).unwrap();
    std::fs::write(iso.join("src/created.txt"), "new\n").unwrap();
    std::fs::write(iso.join("src/staged.txt"), "staged\n").unwrap();
    git(iso, &["add", "src/staged.txt"]);
    std::fs::write(iso.join("owned.txt"), "kept\n").unwrap();
    let paths: Vec<String> = [
        "src/created.txt",
        "src/formatted.txt",
        "src/real.txt",
        "src/staged.txt",
        "src/never-existed.txt",
    ]
    .iter()
    .map(|p| (*p).to_string())
    .collect();
    let dropped = restore_or_remove(iso, &paths);
    assert_eq!(dropped, paths[..4].to_vec());
    assert_eq!(
        std::fs::read_to_string(iso.join("src/real.txt")).unwrap(),
        "fn g() {}\n"
    );
    assert_eq!(
        std::fs::read_to_string(iso.join("src/formatted.txt")).unwrap(),
        "fn f() {\n    1\n}\n"
    );
    assert!(!iso.join("src/created.txt").exists());
    assert!(!iso.join("src/staged.txt").exists());
    assert_eq!(
        std::fs::read_to_string(iso.join("owned.txt")).unwrap(),
        "kept\n",
        "a path not named is never touched"
    );
    assert_eq!(
        super::patch_manifest::workspace_changed_paths(iso).unwrap(),
        vec!["owned.txt".to_string()],
        "capture must see none of the dropped paths"
    );
}
