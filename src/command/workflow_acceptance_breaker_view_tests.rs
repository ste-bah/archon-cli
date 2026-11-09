use super::super::tests::{Fixture, git};
use super::*;
use archon_workflow::base_snapshot::{SnapshotKind, snapshot_deliverables};
use archon_workflow::repository_record::{RepositoryRecordV1, RepositoryTree};
pub(super) fn snapshot(
    f: &Fixture,
    paths: &[&str],
) -> archon_workflow::base_snapshot::BaseSnapshot {
    git(f.source.path(), &["add", "."]);
    git(f.source.path(), &["commit", "-qm", "fixture objects"]);
    let tree = RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: f.source.path().display().to_string(),
        base_commit: git(f.source.path(), &["rev-parse", "HEAD"]),
        decomposition_run_id: "test".into(),
        recorded_at: "test".into(),
    })
    .unwrap();
    snapshot_deliverables(&tree, f.source.path(), paths.iter().copied()).unwrap()
}
#[test]
fn nested_tree_exact_bytes_and_excluded_subtree() {
    let f = Fixture::new("unused");
    std::fs::create_dir_all(f.source.path().join("nested/deeper")).unwrap();
    std::fs::write(f.source.path().join("nested/data"), b"a\r\nb\r\n").unwrap();
    std::fs::write(f.source.path().join("nested/deeper/hidden"), b"excluded").unwrap();
    std::fs::write(
        f.source.path().join(".gitattributes"),
        b"nested/data eol=crlf\n",
    )
    .unwrap();
    let snapshot = snapshot(&f, &["nested"]);
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        &snapshot,
        &[("nested".into(), SnapshotKind::Tree)],
        &["nested/deeper".into()],
    )
    .unwrap();
    assert_eq!(view.files.len(), 1);
    let blob = git(f.source.path(), &["rev-parse", "HEAD:nested/data"]);
    let raw = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(f.source.path())
        .args(["cat-file", "blob", &blob])
        .output()
        .unwrap();
    assert!(raw.status.success());
    assert_eq!(view.files["nested/data"].bytes, raw.stdout);
    assert_eq!(
        std::fs::read(view.root().join("nested/data")).unwrap(),
        raw.stdout
    );
    assert!(!view.root().join("nested/deeper").exists());
}
#[test]
fn size_cap_writes_no_view_files() {
    let f = Fixture::new("unused");
    std::fs::write(
        f.source.path().join("large"),
        vec![b'x'; super::super::super::breaker_view::MAX_VIEW_BYTES + 1],
    )
    .unwrap();
    let snapshot = snapshot(&f, &["large"]);
    let result = materialise_view(
        f.parent.path(),
        f.source.path(),
        &snapshot,
        &[("large".into(), SnapshotKind::Blob)],
        &[],
    );
    assert!(matches!(result,Err(e) if e.0.contains("view_too_large")));
    assert_eq!(std::fs::read_dir(f.parent.path()).unwrap().count(), 0);
}
#[cfg(unix)]
#[test]
fn links_are_skipped_and_dead_owner_views_swept() {
    let f = Fixture::new("unused");
    std::fs::create_dir(f.source.path().join("nested")).unwrap();
    std::os::unix::fs::symlink("../value", f.source.path().join("nested/link")).unwrap();
    std::fs::write(f.source.path().join("nested/file"), b"safe").unwrap();
    let snapshot = snapshot(&f, &["nested"]);
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        &snapshot,
        &[("nested".into(), SnapshotKind::Tree)],
        &[],
    )
    .unwrap();
    assert_eq!(view.skipped, vec!["nested/link"]);
    assert!(!view.root().join("nested/link").exists());
    let root = view.root().to_path_buf();
    std::mem::forget(view);
    std::fs::write(
        owner::marker(&root),
        serde_json::json!({"pid":u32::MAX}).to_string(),
    )
    .unwrap();
    assert!(owner::sweep(f.parent.path()).is_empty());
    assert!(!root.exists());
    assert!(!owner::marker(&root).exists());
}
