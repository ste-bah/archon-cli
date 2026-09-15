//! Issue-25: the dispatch-time trigger consults apply receipts.
use super::*;
use crate::repository_audit::receipts::ApplyReceipt;
use crate::repository_audit::runtime::Snapshot;
use crate::v2::write::audit_refresh::refresh_trigger;

fn snapshot(identity: &str) -> Snapshot {
    Snapshot {
        identity: identity.into(),
        root: PathBuf::from("/nonexistent"),
        paths: vec![],
    }
}

fn receipt(before: &str, after: &str, call_id: &str) -> ApplyReceipt {
    ApplyReceipt {
        commit: "c".into(),
        items_applied: vec!["call-0".into()],
        before: before.into(),
        after: after.into(),
        unexpected_paths: vec![],
        call_id: call_id.into(),
    }
}

#[test]
fn no_audited_snapshot_is_initial() {
    let out = refresh_trigger(
        None,
        &snapshot("y"),
        &[receipt("x", "y", "call")],
        Path::new("/nonexistent"),
    )
    .unwrap();
    assert_eq!(out.trigger, "initial");
    assert!(out.receipt.is_none());
}

#[test]
fn same_tree_is_dispatch() {
    let out = refresh_trigger(
        Some(&snapshot("x")),
        &snapshot("x"),
        &[],
        Path::new("/nonexistent"),
    )
    .unwrap();
    assert_eq!(out.trigger, "dispatch");
}

#[test]
fn tree_named_by_a_receipt_after_is_post_apply() {
    let receipts = [receipt("w", "x", "call"), receipt("x", "y", "")];
    let out = refresh_trigger(
        Some(&snapshot("x")),
        &snapshot("y"),
        &receipts,
        Path::new("/nonexistent"),
    )
    .unwrap();
    assert_eq!(out.trigger, "post_apply");
    assert_eq!(out.receipt.as_ref().map(|r| r.after.as_str()), Some("y"));
    let detail = out.event_detail();
    assert_eq!(detail["apply_receipt"]["before"], "x");
    assert_eq!(detail["unexpected_paths"], serde_json::json!([]));
}

#[test]
fn changed_tree_with_no_receipt_is_unexpected() {
    let out = refresh_trigger(
        Some(&snapshot("x")),
        &snapshot("y"),
        &[],
        Path::new("/nonexistent"),
    )
    .unwrap();
    assert_eq!(out.trigger, "unexpected_change");
    assert!(out.receipt.is_none());
    assert_eq!(out.event_detail(), serde_json::json!({}));
}

#[test]
fn receipt_for_another_tree_without_manifests_is_unexpected() {
    // A pre-Issue-25 receipt (no call id) cannot name manifests, so the
    // content comparison is never attempted against these fake roots.
    let out = refresh_trigger(
        Some(&snapshot("x")),
        &snapshot("y"),
        &[receipt("x", "z", "")],
        Path::new("/nonexistent"),
    )
    .unwrap();
    assert_eq!(out.trigger, "unexpected_change");
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}

/// A committed tree at `root` with `content` in `a.txt`, as a snapshot.
fn tree(root: &Path, content: &str) -> Snapshot {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@local"]);
    std::fs::write(root.join("a.txt"), content).unwrap();
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-qm", "tree"]);
    Snapshot {
        identity: git(root, &["rev-parse", "HEAD^{tree}"]),
        root: root.into(),
        paths: vec!["a.txt".into()],
    }
}

fn applied_manifest(run_root: &Path, post_hash: &str) {
    let manifest = PatchManifest {
        schema: "patch-manifest-v1".into(),
        run_id: "run".into(),
        stage_id: "call".into(),
        item_id: "call-0".into(),
        baseline_commit: "base".into(),
        patch_path: PathBuf::from("call-0.patch"),
        declared_target_files: vec![],
        changed_files: vec!["a.txt".into()],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::from([("a.txt".to_string(), post_hash.to_string())]),
        verify_command: None,
        agent_artifact_path: None,
        status: ManifestStatus::Applied,
        skipped_ignored: BTreeMap::new(),
    };
    let path = PathBuf::from(manifest_path_for(run_root, "call", "call-0"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[test]
fn every_differing_path_accounted_for_by_a_receipts_manifest_is_post_apply() {
    let temp = tempfile::tempdir().unwrap();
    let previous = tree(&temp.path().join("x"), "one\n");
    let current = tree(&temp.path().join("y"), "two\n");
    let run_root = temp.path().join("run");
    applied_manifest(&run_root, blake3::hash(b"two\n").to_hex().as_str());
    // The receipt's `after` is not this tree (its audit captured a different
    // path set), yet its manifests explain every difference.
    let receipts = [receipt(&previous.identity, "elsewhere", "call")];
    let out = refresh_trigger(Some(&previous), &current, &receipts, &run_root).unwrap();
    assert_eq!(out.trigger, "post_apply");
}

#[test]
fn a_differing_path_no_manifest_explains_is_unexpected() {
    let temp = tempfile::tempdir().unwrap();
    let previous = tree(&temp.path().join("x"), "one\n");
    let current = tree(&temp.path().join("y"), "two\n");
    let run_root = temp.path().join("run");
    applied_manifest(
        &run_root,
        blake3::hash(b"something else\n").to_hex().as_str(),
    );
    let receipts = [receipt(&previous.identity, "elsewhere", "call")];
    let out = refresh_trigger(Some(&previous), &current, &receipts, &run_root).unwrap();
    assert_eq!(out.trigger, "unexpected_change");
}
