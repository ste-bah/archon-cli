//! TASK-WC-005 tests (child module via #[path]; file-size guard).
//! Capture tests shell out to git; validation tests build CapturedPatch directly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::*;
use crate::write_coordinator::worktree_isolation::{
    capture_canonical_baseline, create_item_workspace,
};
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
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "t"], root);
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
    WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/run1/impl-0"),
        target_files: targets.iter().map(|t| np(t, root)).collect(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: BTreeSet::new(),
    }
}

fn cfg() -> WriteCoordinatorConfig {
    WriteCoordinatorConfig::default()
}

/// Build canonical repo + isolated workspace, returning (tempdir, plan, workspace, baseline).
fn isolate(
    targets: &[&str],
) -> (
    tempfile::TempDir,
    WritePlan,
    crate::write_coordinator::worktree_isolation::ItemWorkspace,
    crate::write_coordinator::worktree_isolation::CanonicalBaseline,
) {
    let repo = canonical_repo();
    let plan = plan_for(repo.path(), targets);
    let baseline = capture_canonical_baseline(repo.path(), &plan, &[], &cfg()).expect("baseline");
    let ws = create_item_workspace(repo.path(), &plan, &baseline).expect("workspace");
    (repo, plan, ws, baseline)
}

#[test]
fn single_file_edit_capture() {
    let (_repo, plan, ws, baseline) = isolate(&["src/lib.rs"]);
    std::fs::write(plan.isolated_root.join("src/lib.rs"), "// edited\n").expect("edit");
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    assert_eq!(captured.changed_files, vec!["src/lib.rs".to_string()]);
    assert!(captured.created_files.is_empty());
    assert!(captured.deleted_files.is_empty());
    assert!(!captured.patch_bytes.is_empty());
}

#[test]
fn new_file_create_capture() {
    let (_repo, plan, ws, baseline) = isolate(&["src/new.rs"]);
    std::fs::write(plan.isolated_root.join("src/new.rs"), "// created\n").expect("create");
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    assert_eq!(captured.created_files, vec!["src/new.rs".to_string()]);
    let text = String::from_utf8_lossy(&captured.patch_bytes);
    assert!(text.contains("--- /dev/null"));
    assert!(text.contains("+++ b/src/new.rs"));
}

#[test]
fn delete_capture_sets_sentinel() {
    let (_repo, plan, ws, baseline) = isolate(&["src/lib.rs"]);
    std::fs::remove_file(plan.isolated_root.join("src/lib.rs")).expect("delete");
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    assert_eq!(captured.deleted_files, vec!["src/lib.rs".to_string()]);
    assert_eq!(captured.post_hashes.get("src/lib.rs").unwrap(), "deleted");
}

#[test]
fn staged_and_unstaged_single_hunk() {
    let (_repo, plan, ws, baseline) = isolate(&["src/lib.rs"]);
    let path = plan.isolated_root.join("src/lib.rs");
    std::fs::write(&path, "// staged change\n").expect("edit1");
    git(&["add", "src/lib.rs"], &plan.isolated_root);
    std::fs::write(&path, "// staged then unstaged\n").expect("edit2");
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    let text = String::from_utf8_lossy(&captured.patch_bytes);
    let hunks = text.matches("@@ ").count();
    assert_eq!(hunks, 1, "combined change must yield exactly one hunk, not double-counted");
}

#[test]
fn path_with_space_in_directory_preserved() {
    // Sherlock Gate-3 Defect A regression: a path under a space-containing dir
    // must survive capture intact (NUL-separated --name-status).
    let (_repo, plan, ws, baseline) = isolate(&["a b/c.rs"]);
    std::fs::create_dir_all(plan.isolated_root.join("a b")).expect("mkdir");
    std::fs::write(plan.isolated_root.join("a b/c.rs"), "// new\n").expect("create");
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    assert_eq!(captured.changed_files, vec!["a b/c.rs".to_string()]);
    assert_eq!(captured.created_files, vec!["a b/c.rs".to_string()]);
}

#[test]
fn rename_recorded_as_delete_plus_create() {
    // Sherlock Gate-3 Defect B regression: with --no-renames a move is a
    // delete of the old path plus a create of the new, both accounted.
    let (_repo, plan, ws, baseline) = isolate(&["src/lib.rs", "src/renamed.rs"]);
    git(&["mv", "src/lib.rs", "src/renamed.rs"], &plan.isolated_root);
    let captured = capture_patch(&ws, &plan.target_files, &baseline).expect("capture");
    assert!(captured.deleted_files.contains(&"src/lib.rs".to_string()));
    assert!(captured.created_files.contains(&"src/renamed.rs".to_string()));
    assert_eq!(captured.post_hashes.get("src/lib.rs").unwrap(), "deleted");
}

fn manual_capture(patch: &[u8], changed: &[&str]) -> CapturedPatch {
    CapturedPatch {
        patch_bytes: patch.to_vec(),
        changed_files: changed.iter().map(|s| s.to_string()).collect(),
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::new(),
        baseline_commit: "abc".into(),
    }
}

#[test]
fn undeclared_write_rejected() {
    let repo = canonical_repo();
    let plan = plan_for(repo.path(), &["src/lib.rs"]);
    std::fs::create_dir_all(plan.isolated_root.join("src")).ok();
    let captured = manual_capture(b"+x\n", &["src/other.rs"]);
    match validate_patch(&captured, &plan, &cfg(), "ok body") {
        Err(PatchError::UndeclaredWrite { path }) => assert_eq!(path, "src/other.rs"),
        other => panic!("expected UndeclaredWrite, got {other:?}"),
    }
}

#[test]
fn patch_too_large_rejected() {
    let repo = canonical_repo();
    let plan = plan_for(repo.path(), &["src/lib.rs"]);
    let big = vec![b'+'; 100];
    let captured = manual_capture(&big, &["src/lib.rs"]);
    let c = WriteCoordinatorConfig {
        max_patch_bytes: 10,
        ..cfg()
    };
    match validate_patch(&captured, &plan, &c, "ok") {
        Err(PatchError::PatchTooLarge { size, max }) => {
            assert_eq!(size, 100);
            assert_eq!(max, 10);
        }
        other => panic!("expected PatchTooLarge, got {other:?}"),
    }
}

#[test]
fn file_too_large_rejected() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    std::fs::write(plan.isolated_root.join("src/lib.rs"), vec![b'x'; 4096]).expect("big file");
    let captured = manual_capture(b"+small patch\n", &["src/lib.rs"]);
    let c = WriteCoordinatorConfig {
        max_file_bytes: 1024,
        ..cfg()
    };
    match validate_patch(&captured, &plan, &c, "ok") {
        Err(PatchError::FileTooLarge { path, size, max }) => {
            assert_eq!(path, "src/lib.rs");
            assert_eq!(size, 4096);
            assert_eq!(max, 1024);
        }
        other => panic!("expected FileTooLarge, got {other:?}"),
    }
}

#[test]
fn anthropic_secret_rejected() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    let patch = b"@@\n+const KEY = \"sk-ant-FAKEKEYFOR40CHARS0123456789ABCDEFGHIJKLMN\"\n";
    let captured = manual_capture(patch, &["src/lib.rs"]);
    match validate_patch(&captured, &plan, &cfg(), "ok") {
        Err(PatchError::SecretDetected { rule, .. }) => assert_eq!(rule, "anthropic_api_key"),
        other => panic!("expected SecretDetected, got {other:?}"),
    }
}

#[test]
fn private_key_secret_rejected() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    let patch = b"@@\n+-----BEGIN OPENSSH PRIVATE KEY-----\n";
    let captured = manual_capture(patch, &["src/lib.rs"]);
    match validate_patch(&captured, &plan, &cfg(), "ok") {
        Err(PatchError::SecretDetected { rule, .. }) => assert_eq!(rule, "private_key_pem"),
        other => panic!("expected SecretDetected, got {other:?}"),
    }
}

#[test]
fn benign_sk_prefix_passes() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    std::fs::write(plan.isolated_root.join("src/lib.rs"), "// edited\n").ok();
    let patch = b"@@\n+let name = \"sk-input-config\";\n";
    let captured = manual_capture(patch, &["src/lib.rs"]);
    validate_patch(&captured, &plan, &cfg(), "ok").expect("benign sk- prefix must pass");
}

#[test]
fn empty_patch_idempotent_noop_ok() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    let mut captured = manual_capture(b"", &[]);
    captured.pre_hashes.insert("src/lib.rs".into(), "h".into());
    captured.post_hashes.insert("src/lib.rs".into(), "h".into());
    validate_patch(&captured, &plan, &cfg(), r#"{"idempotent_noop": true}"#)
        .expect("declared idempotent noop with matching hashes is ok");
}

#[test]
fn empty_patch_without_noop_rejected() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    let captured = manual_capture(b"", &[]);
    match validate_patch(&captured, &plan, &cfg(), "just text") {
        Err(PatchError::EmptyPatch) => {}
        other => panic!("expected EmptyPatch, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn symlink_escape_rejected() {
    let canonical = tempfile::tempdir().expect("canonical");
    let isolated = tempfile::tempdir().expect("isolated");
    let outside = tempfile::tempdir().expect("outside");
    std::os::unix::fs::symlink(outside.path().join("secret"), isolated.path().join("link.txt"))
        .expect("symlink");
    let mut plan = plan_for(canonical.path(), &["link.txt"]);
    plan.isolated_root = isolated.path().to_path_buf();
    let captured = manual_capture(b"+x\n", &["link.txt"]);
    match validate_patch(&captured, &plan, &cfg(), "ok") {
        Err(PatchError::SymlinkEscape { path }) => assert_eq!(path, "link.txt"),
        other => panic!("expected SymlinkEscape, got {other:?}"),
    }
}

#[test]
fn output_not_usable_rejected() {
    let (_repo, plan, _ws, _baseline) = isolate(&["src/lib.rs"]);
    std::fs::write(plan.isolated_root.join("src/lib.rs"), "// edited\n").ok();
    let captured = manual_capture(b"@@\n+// edited\n", &["src/lib.rs"]);
    // A body that self-reports blocked must be rejected by ensure_output_usable.
    let blocked = "**Status:** blocked\nMissing required evidence.";
    match validate_patch(&captured, &plan, &cfg(), blocked) {
        Err(PatchError::OutputNotUsable { .. }) => {}
        other => panic!("blocked body must surface OutputNotUsable, got {other:?}"),
    }
}

#[test]
fn persist_writes_manifest_and_patch_atomically() {
    let run_root = tempfile::tempdir().expect("run root");
    let item = ItemId::from("impl-0");
    let captured = manual_capture(b"@@\n+x\n", &["src/lib.rs"]);
    let path = persist_manifest(
        run_root.path(),
        "run1",
        "impl",
        &item,
        &captured,
        ManifestStatus::PendingApply,
    )
    .expect("persist");
    assert!(path.exists());
    let manifest_json = run_root
        .path()
        .join("write-coordination/stages/impl/manifests/impl-0.json");
    let patch_file = run_root
        .path()
        .join("write-coordination/stages/impl/patches/impl-0.patch");
    assert!(manifest_json.exists());
    assert!(patch_file.exists());
    assert_eq!(std::fs::read(&patch_file).unwrap(), b"@@\n+x\n");
}

#[test]
fn status_update_rewrites_json_only() {
    let run_root = tempfile::tempdir().expect("run root");
    let item = ItemId::from("impl-0");
    let captured = manual_capture(b"@@\n+x\n", &["src/lib.rs"]);
    persist_manifest(
        run_root.path(),
        "run1",
        "impl",
        &item,
        &captured,
        ManifestStatus::PendingApply,
    )
    .expect("persist");
    let patch_file = run_root
        .path()
        .join("write-coordination/stages/impl/patches/impl-0.patch");
    let patch_before = std::fs::read(&patch_file).unwrap();

    let manifest = PatchManifest {
        schema: PATCH_MANIFEST_SCHEMA.into(),
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: item.clone(),
        baseline_commit: "abc".into(),
        patch_path: patch_file.clone(),
        declared_target_files: vec!["src/lib.rs".into()],
        changed_files: vec!["src/lib.rs".into()],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::new(),
        verify_command: None,
        agent_artifact_path: None,
        status: ManifestStatus::Applied,
    };
    persist_manifest_status_update(run_root.path(), "run1", "impl", &item, &manifest)
        .expect("status update");
    assert_eq!(
        std::fs::read(&patch_file).unwrap(),
        patch_before,
        "patch file must be byte-identical after status update"
    );
    let json = std::fs::read_to_string(
        run_root
            .path()
            .join("write-coordination/stages/impl/manifests/impl-0.json"),
    )
    .unwrap();
    assert!(json.contains("applied"));
}
