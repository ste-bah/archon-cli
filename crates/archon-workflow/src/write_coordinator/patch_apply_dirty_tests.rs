use std::collections::BTreeMap;
use std::path::Path;

use super::*;
use crate::write_coordinator::patch_manifest::{capture_patch, persist_manifest};
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace,
};
use crate::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
use crate::write_coordinator::{ItemId, WriteCoordinatorConfig, WritePlan};

fn git(args: &[&str], cwd: &Path) -> Vec<u8> {
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
    out.stdout
}

fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    // Line endings pinned: Git for Windows defaults to core.autocrlf=true,
    // so a file committed with LF is checked back out with CRLF and every
    // byte-exact content assertion below fails on that platform only.
    git(&["config", "core.autocrlf", "false"], root);
    git(&["config", "core.eol", "lf"], root);
    git(&["config", "user.name", "t"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src/lib.rs"), "// original\n").expect("write");
    git(&["add", "-A"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

fn prepare(repo: &Path, item: &str, edit: &str) -> (PatchManifest, BTreeMap<String, String>) {
    let plan = WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from(item),
        canonical_root: repo.to_path_buf(),
        isolated_root: repo.join(".archon/wc/run1").join(item),
        target_files: vec![normalize_target("src/lib.rs", repo).unwrap()],
        target_dir_scopes: Vec::new(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    };
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(repo, &plan, &[], &cfg).expect("baseline");
    let ws = create_item_workspace(repo, &plan, &baseline).expect("workspace");
    std::fs::write(plan.isolated_root.join("src/lib.rs"), edit).expect("edit");
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    let run_root = repo.join(".archon/workflows/run1");
    persist_manifest(
        &run_root,
        "run1",
        "impl",
        &plan.item_id,
        &captured,
        ManifestStatus::PendingApply,
    )
    .expect("persist");
    let text = std::fs::read_to_string(
        run_root
            .join("write-coordination/stages/impl/manifests")
            .join(format!("{item}.json")),
    )
    .unwrap();
    (serde_json::from_str(&text).unwrap(), captured.pre_hashes)
}

#[test]
fn staged_dirty_target_applies_without_index_mismatch() {
    let repo = canonical_repo();
    std::fs::write(repo.path().join("src/lib.rs"), "// staged baseline\n").unwrap();
    git(&["add", "src/lib.rs"], repo.path());
    let (manifest, pre_hashes) =
        prepare(repo.path(), "impl-0", "// staged baseline\n// agent edit\n");
    let mut pre_by_item = BTreeMap::new();
    pre_by_item.insert(manifest.item_id.clone(), pre_hashes);

    let rec = apply_wave(
        repo.path(),
        std::slice::from_ref(&manifest),
        &pre_by_item,
        0,
        &repo.path().join(".archon/workflows/run1"),
        "run1",
        "impl",
    )
    .expect("apply");

    assert_eq!(rec.items_applied, vec!["impl-0".to_string()]);
    assert!(rec.items_failed.is_empty());
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap(),
        "// staged baseline\n// agent edit\n"
    );
    let cached = git(
        &["diff", "--cached", "--name-only", "--", "src/lib.rs"],
        repo.path(),
    );
    assert_eq!(String::from_utf8_lossy(&cached).trim(), "src/lib.rs");
}
