use super::*;
fn code(outcome: RunOutcome) -> String {
    match outcome {
        RunOutcome::Inapplicable { code, .. } | RunOutcome::Unrun { code, .. } => code,
        other => panic!("{other:?}"),
    }
}
#[test]
fn rejects_non_modifications_and_invalid_paths() {
    let f = Fixture::new("true");
    let c = f.copy();
    let file = f.parent.path().join("bad.patch");
    for (patch, expected) in [
        (
            "diff --git a/new b/new\nnew file mode 100644\n--- /dev/null\n+++ b/new\n@@ -0,0 +1 @@\n+new\n",
            "patch_not_modify_only",
        ),
        (
            "diff --git a/value b/value\ndeleted file mode 100644\n--- a/value\n+++ /dev/null\n@@ -1 +0,0 @@\n-good\n",
            "patch_not_modify_only",
        ),
        (
            "diff --git a/value b/new\nsimilarity index 100%\nrename from value\nrename to new\n",
            "patch_not_modify_only",
        ),
        (
            "diff --git a/value b/value\nold mode 100644\nnew mode 100755\n",
            "patch_not_modify_only",
        ),
        (
            include_str!("../../tests/fixtures/breakage/binary.patch"),
            "patch_binary",
        ),
        (
            "diff --git a/../x b/../x\n--- a/../x\n+++ b/../x\n@@ -1 +1 @@\n-a\n+b\n",
            "patch_path_unsafe",
        ),
        (
            "diff --git a/missing b/missing\n--- a/missing\n+++ b/missing\n@@ -1 +1 @@\n-good\n+evil\n",
            "patch_does_not_apply",
        ),
    ] {
        assert_eq!(
            code(patch::apply(c.copy(), &file, patch.as_bytes()).unwrap_err()),
            expected,
            "{patch}"
        );
        assert_eq!(
            std::fs::read(c.repository().join("value")).unwrap(),
            b"good\n"
        );
    }
}
#[test]
fn prefix_translation_and_failed_apply() {
    let f = Fixture::new("true");
    std::fs::create_dir(f.source.path().join("sub")).unwrap();
    std::fs::write(f.source.path().join("sub/value"), b"good\n").unwrap();
    git(f.source.path(), &["add", "."]);
    git(f.source.path(), &["commit", "-qm", "sub"]);
    let base = git(f.source.path(), &["rev-parse", "HEAD"]);
    let root = f.source.path().join("sub");
    let tree = RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: root.display().to_string(),
        base_commit: base.clone(),
        decomposition_run_id: "test".into(),
        recorded_at: "test".into(),
    })
    .unwrap();
    let snapshot = snapshot_deliverables(&tree, &root, ["value"]).unwrap();
    let c = materialise_base_copy(
        f.parent.path(),
        &root,
        &root,
        None,
        &snapshot,
        &base,
        &f.parent.path().join("output"),
        None,
    )
    .unwrap();
    let file = f.parent.path().join("sub.patch");
    let bytes = f.mutants.mutants[0].patch.as_bytes();
    let inventory = patch::inventory(c.copy()).unwrap();
    let touched = patch::apply(c.copy(), &file, bytes).unwrap();
    assert_eq!(touched, vec!["value"]);
    assert_eq!(
        std::fs::read(c.repository().join("value")).unwrap(),
        b"evil\n"
    );
    patch::restore(c.copy(), &file, &touched, &inventory).unwrap();
    assert_eq!(
        std::fs::read(c.repository().join("value")).unwrap(),
        b"good\n"
    );
    let bad = String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace("-good", "-xxxx");
    assert_eq!(
        code(patch::apply(c.copy(), &file, bad.as_bytes()).unwrap_err()),
        "patch_does_not_apply"
    );
}
#[test]
fn sweep_dead_own_orphan_and_unmarked() {
    let parent = tempfile::tempdir().unwrap();
    let dead = parent.path().join(format!("{COPY_PREFIX}dead"));
    std::fs::create_dir(&dead).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "no_such_test"])
        .spawn()
        .unwrap();
    let pid = child.id();
    child.wait().unwrap();
    std::fs::write(
        owner::marker(&dead),
        serde_json::json!({"pid":pid}).to_string(),
    )
    .unwrap();
    let own = parent.path().join(format!("{COPY_PREFIX}own"));
    std::fs::create_dir(&own).unwrap();
    owner::write(&own).unwrap();
    let unmarked = parent.path().join(format!("{COPY_PREFIX}unmarked"));
    std::fs::create_dir(&unmarked).unwrap();
    let orphan = parent.path().join(format!("{COPY_PREFIX}orphan.owner"));
    std::fs::write(&orphan, b"{}").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&orphan)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(90000))
        .unwrap();
    assert!(sweep(parent.path()).is_empty());
    assert!(!dead.exists());
    assert!(!owner::marker(&dead).exists());
    assert!(own.exists());
    assert!(unmarked.exists());
    assert!(!orphan.exists());
}
#[test]
fn no_change_crlf_mismatch_and_attribute_conversion() {
    let f = Fixture::new("true");
    let c = f.copy();
    let file = f.parent.path().join("same.patch");
    let no_change = b"diff --git a/value b/value\n";
    assert_eq!(
        code(patch::apply(c.copy(), &file, no_change).unwrap_err()),
        "patch_no_change"
    );
    for attributes in [false, true] {
        let f = Fixture::new("true");
        std::fs::write(
            f.source.path().join("value"),
            if attributes {
                b"good\n".as_slice()
            } else {
                b"good\r\n".as_slice()
            },
        )
        .unwrap();
        if attributes {
            std::fs::write(f.source.path().join(".gitattributes"), b"value eol=crlf\n").unwrap();
        }
        git(f.source.path(), &["add", "."]);
        git(f.source.path(), &["commit", "-qm", "line endings"]);
        let base = git(f.source.path(), &["rev-parse", "HEAD"]);
        let tree = RepositoryTree::load(&RepositoryRecordV1 {
            schema_version: 1,
            repository_root: f.source.path().display().to_string(),
            base_commit: base.clone(),
            decomposition_run_id: "test".into(),
            recorded_at: "test".into(),
        })
        .unwrap();
        let snapshot = snapshot_deliverables(&tree, f.source.path(), ["value"]).unwrap();
        let c = materialise_base_copy(
            f.parent.path(),
            f.source.path(),
            f.source.path(),
            None,
            &snapshot,
            &base,
            &f.parent.path().join("output"),
            None,
        )
        .unwrap();
        let before = std::fs::read(c.repository().join("value")).unwrap();
        assert_eq!(before, b"good\r\n");
        let file = f.parent.path().join("lf.patch");
        let inventory = patch::inventory(c.copy()).unwrap();
        let applied = patch::apply(c.copy(), &file, f.mutants.mutants[0].patch.as_bytes());
        if attributes {
            let touched = applied.unwrap();
            patch::restore(c.copy(), &file, &touched, &inventory).unwrap();
            assert_eq!(std::fs::read(c.repository().join("value")).unwrap(), before);
        } else {
            assert_eq!(code(applied.unwrap_err()), "patch_does_not_apply");
        }
    }
}
#[test]
fn case_only_paths_are_verified_and_mtime_failure_is_typed() {
    let f = Fixture::new("true");
    let c = f.copy();
    let file = f.parent.path().join("case.patch");
    if c.repository().join("VALUE").exists() {
        let inventory = patch::inventory(c.copy()).unwrap();
        let text = f.mutants.mutants[0].patch.replace("value", "VALUE");
        assert_eq!(
            code(patch::apply(c.copy(), &file, text.as_bytes()).unwrap_err()),
            "apply_unverified"
        );
        patch::restore(c.copy(), &file, &[], &inventory).unwrap();
        assert_eq!(
            std::fs::read(c.repository().join("value")).unwrap(),
            b"good\n"
        );
    } else {
        eprintln!("SKIP: case-insensitive filesystem unavailable");
    }
    assert_eq!(
        code(patch::bump(c.copy(), &["absent".into()]).unwrap_err()),
        "mtime_not_set"
    );
}
#[cfg(unix)]
#[test]
fn cleanup_does_not_follow_links() {
    let f = Fixture::new("true");
    let c = f.copy();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep"), b"outside").unwrap();
    let before = patch::inventory(c.copy()).unwrap();
    std::os::unix::fs::symlink(outside.path(), c.repository().join("link")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("keep"),
        c.repository().join("file-link"),
    )
    .unwrap();
    patch::cleanup(c.copy(), &before).unwrap();
    assert!(!c.repository().join("link").exists());
    assert!(!c.repository().join("file-link").exists());
    assert_eq!(
        std::fs::read(outside.path().join("keep")).unwrap(),
        b"outside"
    );
}

#[test]
fn future_mtime_remains_monotone_across_both_writes() {
    let f = Fixture::new("true");
    let c = f.copy();
    let path = c.repository().join("value");
    let old = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let before = patch::inventory(c.copy()).unwrap();
    let file = f.parent.path().join("future.patch");
    let touched = patch::apply(c.copy(), &file, f.mutants.mutants[0].patch.as_bytes()).unwrap();
    let applied = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert!(applied >= old + std::time::Duration::from_secs(2));
    patch::restore(c.copy(), &file, &touched, &before).unwrap();
    assert!(
        std::fs::metadata(path).unwrap().modified().unwrap()
            >= applied + std::time::Duration::from_secs(2)
    );
}
#[cfg(unix)]
#[test]
fn linked_patch_paths_are_refused_without_touching_the_destination() {
    let f = Fixture::new("true");
    let outside = tempfile::tempdir().unwrap();
    let destination = outside.path().join("keep");
    std::fs::write(&destination, b"outside").unwrap();
    std::os::unix::fs::symlink(&destination, f.source.path().join("link")).unwrap();
    git(f.source.path(), &["add", "link"]);
    git(f.source.path(), &["commit", "-qm", "link"]);
    let base = git(f.source.path(), &["rev-parse", "HEAD"]);
    let tree = RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: f.source.path().display().to_string(),
        base_commit: base.clone(),
        decomposition_run_id: "test".into(),
        recorded_at: "test".into(),
    })
    .unwrap();
    let snapshot = snapshot_deliverables(&tree, f.source.path(), ["link"]).unwrap();
    std::fs::remove_file(f.source.path().join("link")).unwrap();
    std::os::unix::fs::symlink("other", f.source.path().join("link")).unwrap();
    let bytes = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(f.source.path())
        .args(["diff", "--no-ext-diff"])
        .output()
        .unwrap()
        .stdout;
    let c = materialise_base_copy(
        f.parent.path(),
        f.source.path(),
        f.source.path(),
        None,
        &snapshot,
        &base,
        &f.parent.path().join("output"),
        None,
    )
    .unwrap();
    let old = std::fs::metadata(&destination).unwrap().modified().unwrap();
    assert_eq!(
        code(patch::apply(c.copy(), &f.parent.path().join("link.patch"), &bytes).unwrap_err()),
        "patch_path_unsafe"
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"outside");
    assert_eq!(
        std::fs::metadata(destination).unwrap().modified().unwrap(),
        old
    );
}

#[test]
fn unapplied_patch_is_never_reversed_during_cleanup() {
    for replacement in ["evil", "nope", "gone"] {
        let f = Fixture::new("true");
        let c = f.copy();
        let before = patch::inventory(c.copy()).unwrap();
        let file = f.parent.path().join("unapplied.patch");
        let bytes = f.mutants.mutants[0]
            .patch
            .replace("-good", &format!("-{replacement}"))
            .replace("+evil", "+good");
        assert!(matches!(
            patch::apply(c.copy(), &file, bytes.as_bytes()),
            Err(RunOutcome::Inapplicable { .. })
        ));
        assert_eq!(patch::restore(c.copy(), &file, &[], &before).unwrap(), 0);
        assert_eq!(
            std::fs::read(c.repository().join("value")).unwrap(),
            b"good\n"
        );
    }
}
