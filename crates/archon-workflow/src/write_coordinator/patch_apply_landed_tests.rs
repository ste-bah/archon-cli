//! Issue-25: after apply, `post_hashes` proves every file the wave landed,
//! not only the declared targets.
//!
//! Live on wf-719ff3b0 `agents-7-0`, `providers/tradingview_store.rs` was
//! created under the directory scope of a declared `providers/mod.rs`: in
//! `created_files`, in the wave commit, but with no post-hash, so the
//! post-apply audit reported the wave's own file as an unexpected change and
//! burnt the refresh allowance.

use std::collections::BTreeMap;
use std::path::Path;

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

fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "core.autocrlf", "false"], root);
    git(&["config", "core.eol", "lf"], root);
    git(&["config", "user.name", "t"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src/providers")).expect("mkdir");
    std::fs::write(root.join("src/providers/mod.rs"), "// original\n").expect("write");
    std::fs::write(root.join("src/providers/old.rs"), "// old\n").expect("write");
    git(&["add", "-A"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

/// One declared file (`src/providers/mod.rs`) with its directory as scope;
/// the branch edits it, creates `store.rs` beside it and deletes `old.rs`.
fn prepare(repo: &Path) -> (PatchManifest, BTreeMap<String, String>) {
    let plan = WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: repo.to_path_buf(),
        isolated_root: repo.join(".archon/wc/run1/impl-0"),
        target_files: vec![normalize_target("src/providers/mod.rs", repo).unwrap()],
        target_dir_scopes: vec![normalize_target("src/providers", repo).unwrap()],
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
    std::fs::write(
        plan.isolated_root.join("src/providers/mod.rs"),
        "mod store;\n",
    )
    .unwrap();
    std::fs::write(
        plan.isolated_root.join("src/providers/store.rs"),
        "// store\n",
    )
    .unwrap();
    std::fs::remove_file(plan.isolated_root.join("src/providers/old.rs")).unwrap();
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
        run_root.join("write-coordination/stages/impl/manifests/impl-0.json"),
    )
    .unwrap();
    (serde_json::from_str(&text).unwrap(), captured.pre_hashes)
}

fn blake3_of(path: &Path) -> String {
    blake3::hash(&std::fs::read(path).unwrap())
        .to_hex()
        .to_string()
}

#[test]
fn applied_post_hashes_cover_every_changed_and_created_path_but_not_deleted_ones() {
    let repo = canonical_repo();
    let (manifest, pre_hashes) = prepare(repo.path());
    // The shape under test: landed inside the scope, never declared.
    assert_eq!(
        manifest.declared_target_files,
        vec!["src/providers/mod.rs".to_string()]
    );
    assert!(
        manifest
            .created_files
            .contains(&"src/providers/store.rs".to_string()),
        "{manifest:#?}"
    );
    assert!(
        manifest
            .deleted_files
            .contains(&"src/providers/old.rs".to_string()),
        "{manifest:#?}"
    );
    assert!(
        !manifest.post_hashes.contains_key("src/providers/store.rs"),
        "captured hashes are declared-only"
    );
    let mut pre_by_item = BTreeMap::new();
    pre_by_item.insert(manifest.item_id.clone(), pre_hashes);
    let run_root = repo.path().join(".archon/workflows/run1");
    let rec = apply_wave(
        repo.path(),
        std::slice::from_ref(&manifest),
        &pre_by_item,
        0,
        &run_root,
        "run1",
        "impl",
    )
    .expect("apply");
    assert_eq!(rec.items_applied, vec!["impl-0".to_string()], "{rec:#?}");
    let applied: PatchManifest = serde_json::from_str(
        &std::fs::read_to_string(
            run_root.join("write-coordination/stages/impl/manifests/impl-0.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(applied.status, ManifestStatus::Applied);
    for path in applied.changed_files.iter().chain(&applied.created_files) {
        if applied.deleted_files.contains(path) {
            continue;
        }
        assert_eq!(
            applied.post_hashes.get(path).map(String::as_str),
            Some(blake3_of(&repo.path().join(path)).as_str()),
            "{path} landed without a post-hash: {:?}",
            applied.post_hashes
        );
    }
    assert!(
        applied.post_hashes.contains_key("src/providers/store.rs"),
        "{:?}",
        applied.post_hashes
    );
    assert!(
        !applied.post_hashes.contains_key("src/providers/old.rs"),
        "{:?}",
        applied.post_hashes
    );
    assert!(!repo.path().join("src/providers/old.rs").exists());
    // Still declared-only in the manifest's own terms: the fix is on the hashes.
    assert_eq!(
        applied.declared_target_files,
        vec!["src/providers/mod.rs".to_string()]
    );
}
