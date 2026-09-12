use super::*;
#[test]
fn ignored_deliverable_stays_out_of_shared_tree_and_is_reported() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join(".gitignore"), "docs/\n.archon/\n").unwrap();
    git(&["add", ".gitignore"], root);
    git(&["commit", "-qm", "ignore generated files"], root);
    let (mut manifest, pre) = prepare(root, "ignored", &["docs/report.md"], &[("docs/report.md", "report bytes")]);
    manifest.status = ManifestStatus::IdempotentNoop;
    let run_root = root.join(".archon/workflows/run1");
    let record = with_repo_lock(root, || apply_wave(root, &[manifest.clone()],
        &BTreeMap::from([("ignored".into(), pre)]), 1, &run_root, "run1", "impl")).unwrap();
    assert!(record.items_failed.is_empty(), "{:?}", record.items_failed);
    assert!(record.items_applied.is_empty());
    assert!(!root.join("docs/report.md").exists(), "ignored output contaminated canonical");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(
        run_root.join("write-coordination/stages/impl/manifests/ignored.json")).unwrap()).unwrap();
    assert_eq!(saved["status"]["status"], "skipped_ignored");
    assert_eq!(std::fs::read(run_root.join("artifacts/ignored-deliverables/impl/ignored/docs/report.md")).unwrap(), b"report bytes");
}
