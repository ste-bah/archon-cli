//! Wave-commit regression tests (child module via #[path]; file-size guard).
//!
//! A wave's worktrees are cut from HEAD, so an accepted wave's declared outputs
//! must land as a commit before the next wave starts. Otherwise a file one task
//! creates is invisible to the task that depends on it, and dependency
//! ordering — the reason waves exist — silently does nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::write_coordinator::patch_manifest::{capture_patch, persist_manifest};
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace,
};
use crate::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
use crate::write_coordinator::{ItemId, WriteCoordinatorConfig, WritePlan};

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

fn git_stdout(args: &[&str], cwd: &Path) -> String {
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "core.autocrlf", "false"], root);
    git(&["config", "core.eol", "lf"], root);
    git(&["config", "user.name", "t"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src/lib.rs"), "// original\n").expect("write");
    std::fs::write(root.join("src/other.rs"), "// other\n").expect("write");
    git(&["add", "-A"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

fn run_root_of(repo: &Path) -> PathBuf {
    repo.join(".archon/workflows/run1")
}

fn plan_for(repo: &Path, item: &str, declared: &[&str]) -> WritePlan {
    WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from(item),
        canonical_root: repo.to_path_buf(),
        isolated_root: repo.join(".archon/wc/run1").join(item),
        target_files: declared
            .iter()
            .map(|t| normalize_target(t, repo).unwrap())
            .collect(),
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    }
}

/// Create the isolated worktree a task would work in, and return its root.
fn workspace_root(repo: &Path, item: &str, declared: &[&str]) -> PathBuf {
    let plan = plan_for(repo, item, declared);
    let baseline = capture_canonical_baseline(repo, &plan, &[], &WriteCoordinatorConfig::default())
        .expect("baseline");
    create_item_workspace(repo, &plan, &baseline).expect("workspace");
    plan.isolated_root
}

fn prepare(
    repo: &Path,
    item: &str,
    declared: &[&str],
    edits: &[(&str, &str)],
) -> (PatchManifest, BTreeMap<String, String>) {
    let plan = plan_for(repo, item, declared);
    let baseline = capture_canonical_baseline(repo, &plan, &[], &WriteCoordinatorConfig::default())
        .expect("baseline");
    let ws = create_item_workspace(repo, &plan, &baseline).expect("workspace");
    for (rel, content) in edits {
        let p = plan.isolated_root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).ok();
        std::fs::write(p, content).expect("edit");
    }
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    let run_root = run_root_of(repo);
    persist_manifest(
        &run_root,
        "run1",
        "impl",
        &plan.item_id,
        &captured,
        ManifestStatus::PendingApply,
    )
    .expect("persist");
    let json = std::fs::read_to_string(
        run_root
            .join("write-coordination/stages/impl/manifests")
            .join(format!("{item}.json")),
    )
    .unwrap();
    let manifest: PatchManifest = serde_json::from_str(&json).expect("parse manifest");
    (manifest, captured.pre_hashes)
}

fn apply_one_item(repo: &Path, manifest: &PatchManifest, pre: BTreeMap<String, String>) {
    let mut by_item = BTreeMap::new();
    by_item.insert(manifest.item_id.clone(), pre);
    apply_wave(
        repo,
        std::slice::from_ref(manifest),
        &by_item,
        0,
        &run_root_of(repo),
        "run1",
        "impl",
    )
    .expect("apply");
}

/// A dependency chain: task B consumes a file task A newly created.
///
/// `.txt` is deliberate — it is outside the untracked support-file allowlist,
/// which is what made this invisible in production.
#[test]
fn a_later_wave_worktree_sees_a_file_an_earlier_wave_created() {
    let repo = canonical_repo();
    let root = repo.path();

    let (manifest, pre) = prepare(
        root,
        "impl-0",
        &["src/produced.txt"],
        &[("src/produced.txt", "PRODUCED\n")],
    );
    apply_one_item(root, &manifest, pre);

    let later = workspace_root(root, "impl-1", &["src/consumer.json"]);
    let carried = later.join("src/produced.txt");
    assert!(
        carried.exists(),
        "a later wave's worktree must contain the file an earlier wave created"
    );
    assert_eq!(
        std::fs::read_to_string(&carried).expect("read carried input"),
        "PRODUCED\n"
    );
}

/// The wave commit must never sweep up unrelated working-tree state.
#[test]
fn a_wave_commit_contains_only_the_declared_write_set() {
    let repo = canonical_repo();
    let root = repo.path();

    // Unrelated WIP that must survive uncommitted: one tracked, one untracked.
    std::fs::write(root.join("src/other.rs"), "// edited elsewhere\n").expect("write");
    std::fs::write(root.join("notes.md"), "unrelated\n").expect("write");

    let (manifest, pre) = prepare(
        root,
        "impl-0",
        &["src/produced.txt"],
        &[("src/produced.txt", "PRODUCED\n")],
    );
    apply_one_item(root, &manifest, pre);

    let committed = git_stdout(&["show", "--name-only", "--pretty=format:", "HEAD"], root);
    let files: Vec<&str> = committed.split_whitespace().collect();
    assert_eq!(
        files,
        vec!["src/produced.txt"],
        "wave commit must contain only the declared write-set"
    );

    let status = git_stdout(&["status", "--porcelain"], root);
    assert!(
        status.contains("src/other.rs"),
        "unrelated tracked WIP must stay uncommitted: {status}"
    );
    assert!(
        status.contains("notes.md"),
        "unrelated untracked file must stay uncommitted: {status}"
    );
}
