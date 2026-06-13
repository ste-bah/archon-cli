//! TASK-WC-006 tests (child module via #[path]; file-size guard).

use std::collections::BTreeMap;
use std::path::Path;

use super::*;
use crate::write_coordinator::patch_manifest::{
    PATCH_MANIFEST_SCHEMA, capture_patch, persist_manifest,
};
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
    git(&["config", "user.name", "t"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(root.join("src/lib.rs"), "// original\n").expect("write");
    std::fs::write(root.join("src/other.rs"), "// other\n").expect("write");
    git(&["add", "-A"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

fn cfg() -> WriteCoordinatorConfig {
    WriteCoordinatorConfig::default()
}

/// Prepare a real captured+persisted manifest by editing `edits` in an isolated
/// worktree of `repo`. Returns the manifest and its pre_hashes map.
fn prepare(
    repo: &Path,
    item: &str,
    declared: &[&str],
    edits: &[(&str, &str)],
) -> (PatchManifest, BTreeMap<String, String>) {
    let plan = WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from(item),
        canonical_root: repo.to_path_buf(),
        isolated_root: repo.join(".archon/wc/run1").join(item),
        target_files: declared
            .iter()
            .map(|t| normalize_target(t, repo).unwrap())
            .collect(),
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    };
    let baseline = capture_canonical_baseline(repo, &plan, &[], &cfg()).expect("baseline");
    let ws = create_item_workspace(repo, &plan, &baseline).expect("workspace");
    for (rel, content) in edits {
        let p = plan.isolated_root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).ok();
        std::fs::write(p, content).expect("edit");
    }
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
    let json = std::fs::read_to_string(
        run_root
            .join("write-coordination/stages/impl/manifests")
            .join(format!("{item}.json")),
    )
    .unwrap();
    let manifest: PatchManifest = serde_json::from_str(&json).expect("parse manifest");
    (manifest, captured.pre_hashes)
}

fn run_root_of(repo: &Path) -> std::path::PathBuf {
    repo.join(".archon/workflows/run1")
}

fn blake3_of(path: &Path) -> String {
    blake3::hash(&std::fs::read(path).unwrap())
        .to_hex()
        .to_string()
}

#[test]
fn single_item_clean_apply() {
    let repo = canonical_repo();
    let (m, pre) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs"],
        &[("src/lib.rs", "// edited\n")],
    );
    let mut pre_by_item = BTreeMap::new();
    pre_by_item.insert(m.item_id.clone(), pre);
    let rec = apply_wave(
        repo.path(),
        &[m.clone()],
        &pre_by_item,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    )
    .expect("apply");
    assert_eq!(rec.items_applied, vec!["impl-0".to_string()]);
    assert!(rec.items_failed.is_empty());
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap(),
        "// edited\n"
    );
    assert_eq!(
        resume_status(&m.item_id, &run_root_of(repo.path()), "impl"),
        ApplyResumeStatus::Applied
    );
}

#[test]
fn two_disjoint_items_both_apply() {
    let repo = canonical_repo();
    let (m0, p0) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs"],
        &[("src/lib.rs", "// a\n")],
    );
    let (m1, p1) = prepare(
        repo.path(),
        "impl-1",
        &["src/other.rs"],
        &[("src/other.rs", "// b\n")],
    );
    let mut pre = BTreeMap::new();
    pre.insert(m0.item_id.clone(), p0);
    pre.insert(m1.item_id.clone(), p1);
    let rec = apply_wave(
        repo.path(),
        &[m1, m0],
        &pre,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    )
    .expect("apply");
    // Sorted by item_id: impl-0 before impl-1.
    assert_eq!(
        rec.items_applied,
        vec!["impl-0".to_string(), "impl-1".to_string()]
    );
    assert!(rec.items_failed.is_empty());
}

#[test]
fn stale_baseline_changed_file_fails_item() {
    let repo = canonical_repo();
    let (m, pre) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs"],
        &[("src/lib.rs", "// edited\n")],
    );
    let mut pre_by_item = BTreeMap::new();
    pre_by_item.insert(m.item_id.clone(), pre);
    // Drift canonical AFTER pre_hashes captured.
    std::fs::write(repo.path().join("src/lib.rs"), "// drifted\n").unwrap();
    let rec = apply_wave(
        repo.path(),
        &[m.clone()],
        &pre_by_item,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    )
    .expect("apply");
    assert!(rec.items_applied.is_empty());
    assert_eq!(rec.items_failed.len(), 1);
    assert!(rec.items_failed[0].1.contains("StaleBaseline"));
    match resume_status(&m.item_id, &run_root_of(repo.path()), "impl") {
        ApplyResumeStatus::Failed(reason) => assert!(reason.contains("stale baseline")),
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn stale_baseline_unchanged_file_ignored() {
    let repo = canonical_repo();
    // Declare two targets, edit only lib.rs.
    let (m, pre) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs", "src/other.rs"],
        &[("src/lib.rs", "// edited\n")],
    );
    let mut pre_by_item = BTreeMap::new();
    pre_by_item.insert(m.item_id.clone(), pre);
    // Drift the UNCHANGED declared target — must not trigger stale.
    std::fs::write(repo.path().join("src/other.rs"), "// drifted other\n").unwrap();
    let rec = apply_wave(
        repo.path(),
        &[m],
        &pre_by_item,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    )
    .expect("apply");
    assert_eq!(rec.items_applied, vec!["impl-0".to_string()]);
}

#[test]
fn patch_conflict_cleans_canonical() {
    let repo = canonical_repo();
    let (m, _pre) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs"],
        &[("src/lib.rs", "// edited\n")],
    );
    // Drift canonical working tree so the patch cannot 3-way merge, but make
    // pre_hashes MATCH the drift so the stale check passes (we want the apply
    // step itself to conflict).
    std::fs::write(repo.path().join("src/lib.rs"), "// drifted conflict\n").unwrap();
    let mut pre_by_item = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    hashes.insert(
        "src/lib.rs".to_string(),
        blake3_of(&repo.path().join("src/lib.rs")),
    );
    pre_by_item.insert(m.item_id.clone(), hashes);
    let rec = apply_wave(
        repo.path(),
        &[m],
        &pre_by_item,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    )
    .expect("apply");
    assert_eq!(rec.items_failed.len(), 1, "apply should conflict");
    // Cleanup restored canonical to HEAD: git diff empty.
    let diff = std::process::Command::new("git")
        .current_dir(repo.path())
        .args(["diff", "--name-only"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&diff.stdout).trim().is_empty(),
        "canonical must be clean after conflict cleanup"
    );
}

#[test]
fn failing_verify_reports_nonzero_no_rollback() {
    let repo = canonical_repo();
    let (m, pre) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs"],
        &[("src/lib.rs", "// edited\n")],
    );
    let mut pre_by_item = BTreeMap::new();
    pre_by_item.insert(m.item_id.clone(), pre);
    apply_wave(
        repo.path(),
        &[m],
        &pre_by_item,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    )
    .expect("apply");
    let verify = run_wave_verify(
        repo.path(),
        Some("exit 3"),
        0,
        &run_root_of(repo.path()),
        "impl",
    );
    match verify {
        Err(ApplyError::VerifyFailed { exit, .. }) => assert_eq!(exit, 3),
        other => panic!("expected VerifyFailed, got {other:?}"),
    }
    // No rollback: the applied patch remains.
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap(),
        "// edited\n"
    );
}

#[test]
fn verify_none_is_noop_success() {
    let repo = canonical_repo();
    let v = run_wave_verify(repo.path(), None, 0, &run_root_of(repo.path()), "impl").expect("noop");
    assert_eq!(v.exit, 0);
    assert!(v.stdout_tail.is_empty());
}

#[test]
fn conflict_graph_violation_before_any_git() {
    let repo = canonical_repo();
    let (mut m0, _) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs"],
        &[("src/lib.rs", "// a\n")],
    );
    let (mut m1, _) = prepare(
        repo.path(),
        "impl-1",
        &["src/lib.rs"],
        &[("src/lib.rs", "// b\n")],
    );
    m0.declared_target_files = vec!["src/lib.rs".into()];
    m1.declared_target_files = vec!["src/lib.rs".into()];
    let pre = BTreeMap::new();
    match apply_wave(
        repo.path(),
        &[m0, m1],
        &pre,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    ) {
        Err(ApplyError::ConflictGraphViolation { conflicting_paths }) => {
            assert!(conflicting_paths.contains(&"src/lib.rs".to_string()));
        }
        other => panic!("expected ConflictGraphViolation, got {other:?}"),
    }
    // Canonical untouched.
    assert_eq!(
        std::fs::read_to_string(repo.path().join("src/lib.rs")).unwrap(),
        "// original\n"
    );
}

#[test]
fn apply_wave_does_not_mutate_caller_slice() {
    let repo = canonical_repo();
    let (m0, p0) = prepare(
        repo.path(),
        "impl-1",
        &["src/lib.rs"],
        &[("src/lib.rs", "// a\n")],
    );
    let (m1, p1) = prepare(
        repo.path(),
        "impl-0",
        &["src/other.rs"],
        &[("src/other.rs", "// b\n")],
    );
    let mut pre = BTreeMap::new();
    pre.insert(m0.item_id.clone(), p0);
    pre.insert(m1.item_id.clone(), p1);
    let slice = vec![m0, m1];
    let order_before: Vec<String> = slice.iter().map(|m| m.item_id.clone()).collect();
    apply_wave(
        repo.path(),
        &slice,
        &pre,
        0,
        &run_root_of(repo.path()),
        "run1",
        "impl",
    )
    .expect("apply");
    let order_after: Vec<String> = slice.iter().map(|m| m.item_id.clone()).collect();
    assert_eq!(
        order_before, order_after,
        "caller slice order must be unchanged"
    );
}

#[test]
fn resume_status_variants() {
    let repo = canonical_repo();
    let (m, _) = prepare(
        repo.path(),
        "impl-0",
        &["src/lib.rs"],
        &[("src/lib.rs", "// a\n")],
    );
    let rr = run_root_of(repo.path());
    assert_eq!(
        resume_status(&ItemId::from("ghost"), &rr, "impl"),
        ApplyResumeStatus::NotPersisted
    );
    assert_eq!(
        resume_status(&m.item_id, &rr, "impl"),
        ApplyResumeStatus::PendingApply
    );
    persist_status(&rr, &m, ManifestStatus::Applied);
    assert_eq!(
        resume_status(&m.item_id, &rr, "impl"),
        ApplyResumeStatus::Applied
    );
    persist_status(&rr, &m, ManifestStatus::IdempotentNoop);
    assert_eq!(
        resume_status(&m.item_id, &rr, "impl"),
        ApplyResumeStatus::IdempotentNoop
    );
    persist_status(&rr, &m, ManifestStatus::Conflicted);
    assert_eq!(
        resume_status(&m.item_id, &rr, "impl"),
        ApplyResumeStatus::Conflicted
    );
    persist_status(
        &rr,
        &m,
        ManifestStatus::Failed {
            reason: "boom".into(),
        },
    );
    match resume_status(&m.item_id, &rr, "impl") {
        ApplyResumeStatus::Failed(reason) => assert_eq!(reason, "boom"),
        other => panic!("expected Failed, got {other:?}"),
    }
}

fn persist_status(run_root: &Path, base: &PatchManifest, status: ManifestStatus) {
    let mut m = base.clone();
    m.schema = PATCH_MANIFEST_SCHEMA.into();
    m.status = status;
    crate::write_coordinator::patch_manifest::persist_manifest_status_update(
        run_root, "run1", "impl", &m.item_id, &m,
    )
    .expect("status update");
}

#[path = "patch_apply_lock_tests.rs"]
mod lock_tests;
