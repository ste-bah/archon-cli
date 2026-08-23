//! C4 — gitignored dependencies materialised into a real worktree.
//!
//! Every test here drives `create_item_workspace`, the production entry point,
//! against a real temporary git repository. Nothing exercises a helper in
//! isolation: an earlier session shipped a fix that was green against a pure
//! function and a no-op in production, so the assertions are made against what
//! is on disk after the real call.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{Mechanism, SkipReason};
use crate::write_coordinator::ItemId;
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::worktree_isolation::{
    ItemWorkspace, WorkspaceStatus, capture_canonical_baseline, cleanup_workspace,
    create_item_workspace,
};
use crate::write_coordinator::write_plan::{
    NormalizedPath, TargetFilesSource, WritePlan, normalize_target,
};

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

fn git_stdout(args: &[&str], cwd: &Path) -> String {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repo with one tracked source file, a tracked `.gitignore`, one ignored
/// FILE and one ignored DIRECTORY with content inside it.
fn repo_with_ignored_deps() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "core.autocrlf", "false"], root);
    git(&["config", "core.eol", "lf"], root);
    git(&["config", "user.name", "tester"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src/lib.rs"), "// original\n").expect("write");
    std::fs::write(root.join(".gitignore"), "vendor/\nconfig.local\n").expect("write");
    git(&["add", "src/lib.rs", ".gitignore"], root);
    git(&["commit", "-q", "-m", "init"], root);

    std::fs::create_dir_all(root.join("vendor/pkg")).expect("mkdir");
    std::fs::write(root.join("vendor/pkg/dep.txt"), "vendored dependency\n").expect("write");
    std::fs::write(root.join("config.local"), "TOKEN=local\n").expect("write");
    dir
}

fn plan_for(root: &Path, targets: &[&str]) -> WritePlan {
    let target_files: Vec<NormalizedPath> = targets
        .iter()
        .map(|t| normalize_target(t, root).unwrap_or_else(|e| panic!("normalize {t}: {e}")))
        .collect();
    WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/run1/impl-0"),
        target_files,
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: BTreeSet::new(),
    }
}

fn isolate_at(root: &Path, isolated_suffix: &str) -> (WritePlan, ItemWorkspace) {
    let mut plan = plan_for(root, &["src/lib.rs"]);
    plan.isolated_root = root.join(format!(".archon/wc/run1/{isolated_suffix}"));
    plan.item_id = ItemId::from(isolated_suffix);
    let baseline = capture_canonical_baseline(root, &plan, &[], &WriteCoordinatorConfig::default())
        .expect("capture");
    let ws = create_item_workspace(root, &plan, &baseline).expect("workspace");
    (plan, ws)
}

fn isolate(root: &Path) -> (WritePlan, ItemWorkspace) {
    isolate_at(root, "impl-0")
}

#[test]
fn gitignored_file_and_directory_are_materialised_into_the_worktree() {
    let repo = repo_with_ignored_deps();
    let (plan, ws) = isolate(repo.path());
    let isolated = plan.isolated_root.as_path();

    assert_eq!(
        std::fs::read_to_string(isolated.join("vendor/pkg/dep.txt")).expect("vendored dep"),
        "vendored dependency\n",
        "an ignored dependency directory must be reachable from the worktree"
    );
    assert_eq!(
        std::fs::read_to_string(isolated.join("config.local")).expect("local config"),
        "TOKEN=local\n",
        "an ignored config file must be reachable from the worktree"
    );

    assert_eq!(
        ws.materialized_ignored.mechanism("vendor"),
        Some(Mechanism::SharedDirectory),
        "directories are shared by symlink: {:?}",
        ws.materialized_ignored
    );
    // The node is REAL and its children are symlinks. A whole-tree symlink
    // named `vendor` would not match the `vendor/` ignore pattern and would
    // surface as `?? vendor` in the worktree's git status.
    assert!(
        std::fs::symlink_metadata(isolated.join("vendor"))
            .expect("vendor entry")
            .file_type()
            .is_dir(),
        "the directory node must be real so a `dir/` ignore pattern still matches it"
    );
    assert!(
        std::fs::symlink_metadata(isolated.join("vendor/pkg"))
            .expect("vendor child")
            .file_type()
            .is_symlink(),
        "immediate children are shared symlinks, which is what makes this O(children)"
    );

    let file_mechanism = ws.materialized_ignored.mechanism("config.local");
    assert!(
        matches!(file_mechanism, Some(Mechanism::Reflink | Mechanism::Copy)),
        "files get a PRIVATE copy, not a share: {file_mechanism:?}"
    );
    assert!(
        !std::fs::symlink_metadata(isolated.join("config.local"))
            .expect("config entry")
            .file_type()
            .is_symlink(),
        "an ignored file must be private to the worktree, never a symlink to canonical"
    );
}

#[test]
fn materialised_ignored_paths_stay_invisible_to_git_in_the_worktree() {
    let repo = repo_with_ignored_deps();
    let (plan, _ws) = isolate(repo.path());

    let status = git_stdout(&["status", "--porcelain"], plan.isolated_root.as_path());
    assert!(
        status.trim().is_empty(),
        "materialised paths must not surface as agent-authored changes, got: {status:?}"
    );
}

#[test]
fn an_untracked_gitignore_leaves_ignored_paths_unmaterialised_and_reported() {
    // `.gitignore` never committed: canonical ignores `vendor/`, the worktree
    // does not, so materialising it would look like agent-authored content.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "tester"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src/lib.rs"), "// original\n").expect("write");
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-q", "-m", "init"], root);
    std::fs::write(root.join(".gitignore"), "vendor/\n").expect("write");
    std::fs::create_dir_all(root.join("vendor")).expect("mkdir");
    std::fs::write(root.join("vendor/dep.txt"), "dep\n").expect("write");

    let (plan, ws) = isolate(root);
    assert_eq!(
        ws.materialized_ignored.skip_reason("vendor"),
        Some(&SkipReason::NotIgnoredInWorktree),
        "the worktree's own git is the authority: {:?}",
        ws.materialized_ignored
    );
    assert!(
        !plan.isolated_root.join("vendor").exists(),
        "a path the worktree does not ignore must not be created there"
    );
}

#[test]
fn a_second_branch_shares_the_directory_but_not_the_file() {
    let repo = repo_with_ignored_deps();
    let root = repo.path();
    let (plan_a, _ws_a) = isolate_at(root, "impl-0");
    let (plan_b, _ws_b) = isolate_at(root, "impl-1");

    // Shared directory: branch A's write reaches canonical and branch B.
    std::fs::write(
        plan_a.isolated_root.join("vendor/pkg/added.txt"),
        "from a\n",
    )
    .expect("write into shared dir");
    assert!(
        root.join("vendor/pkg/added.txt").exists(),
        "a symlinked directory is shared with canonical by construction"
    );
    assert_eq!(
        std::fs::read_to_string(plan_b.isolated_root.join("vendor/pkg/added.txt"))
            .expect("branch b sees it"),
        "from a\n",
        "every branch shares one directory; that is the documented trade"
    );

    // Private file: branch A's write reaches nobody else.
    std::fs::write(
        plan_a.isolated_root.join("config.local"),
        "TOKEN=branch-a\n",
    )
    .expect("write private file");
    assert_eq!(
        std::fs::read_to_string(root.join("config.local")).expect("canonical config"),
        "TOKEN=local\n",
        "an ignored FILE is private, so canonical must be untouched"
    );
    assert_eq!(
        std::fs::read_to_string(plan_b.isolated_root.join("config.local")).expect("b config"),
        "TOKEN=local\n",
        "branch B must not see branch A's edit to a private file"
    );
}

#[test]
fn removing_the_worktree_leaves_the_canonical_ignored_directory_intact() {
    let repo = repo_with_ignored_deps();
    let root = repo.path();
    let (plan, _ws) = isolate(root);
    assert!(plan.isolated_root.join("vendor/pkg/dep.txt").exists());

    let cfg = WriteCoordinatorConfig {
        retain_success_worktrees: false,
        ..WriteCoordinatorConfig::default()
    };
    cleanup_workspace(
        root,
        plan.isolated_root.as_path(),
        WorkspaceStatus::Succeeded,
        &cfg,
    )
    .expect("cleanup");

    assert!(
        !plan.isolated_root.exists(),
        "the worktree itself must be gone"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("vendor/pkg/dep.txt")).expect("canonical dep survives"),
        "vendored dependency\n",
        "removing a worktree must unlink the symlink, never delete through it"
    );
}

#[test]
fn a_source_deleted_after_discovery_is_reported_not_guessed() {
    let repo = repo_with_ignored_deps();
    let root = repo.path();
    // A dangling symlink among the ignored entries reproduces the mid-run
    // deletion case: git lists it, reading it finds nothing behind it.
    std::os::unix::fs::symlink(root.join("gone"), root.join("config.local.link"))
        .expect("dangling link");
    std::fs::write(root.join(".gitignore"), "vendor/\nconfig.local*\n").expect("write");
    git(&["add", ".gitignore"], root);
    git(&["commit", "-q", "-m", "ignore more"], root);

    let (plan, ws) = isolate(root);
    assert_eq!(
        ws.materialized_ignored.mechanism("config.local.link"),
        Some(Mechanism::ReplicatedSymlink),
        "a symlink source is reproduced as the same link, not followed: {:?}",
        ws.materialized_ignored
    );
    let link: PathBuf = plan.isolated_root.join("config.local.link");
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("link exists")
            .file_type()
            .is_symlink(),
        "the link must be recreated verbatim, with its dangling target visible"
    );
    assert!(
        std::fs::read_to_string(&link).is_err(),
        "nothing is invented behind a dangling source"
    );
}

#[test]
fn the_entry_cap_reports_every_path_it_skipped() {
    let repo = repo_with_ignored_deps();
    let root = repo.path();
    let overflow = super::MAX_ENTRIES + 8;
    std::fs::write(root.join(".gitignore"), "vendor/\nconfig.local\nblob_*\n").expect("write");
    git(&["add", ".gitignore"], root);
    git(&["commit", "-q", "-m", "ignore blobs"], root);
    for index in 0..overflow {
        std::fs::write(root.join(format!("blob_{index:04}")), "x\n").expect("write blob");
    }

    let (_plan, ws) = isolate(root);
    assert_eq!(
        ws.materialized_ignored.materialized.len(),
        super::MAX_ENTRIES,
        "the cap must bind exactly, not approximately"
    );
    let capped = ws
        .materialized_ignored
        .skipped
        .iter()
        .filter(|(_, reason)| *reason == SkipReason::EntryLimit)
        .count();
    assert_eq!(
        capped,
        overflow + 2 - super::MAX_ENTRIES,
        "every path over the cap must be named as skipped, not silently dropped: {:?}",
        ws.materialized_ignored.skipped.len()
    );
}
