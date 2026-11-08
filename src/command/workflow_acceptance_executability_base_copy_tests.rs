use super::*;
use archon_workflow::base_snapshot::{SnapshotEntry, snapshot_deliverables};
use archon_workflow::repository_record::{RepositoryRecordV1, RepositoryTree};
fn command(root: &Path, args: &[&str]) -> std::process::Output {
    archon_shell::spawn::command("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
pub(super) fn git(root: &Path, args: &[&str]) -> String {
    let result = command(root, args);
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8_lossy(&result.stdout).trim().into()
}
pub(super) fn write(root: &Path, key: &str, bytes: &[u8]) {
    let path = root.join(key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
pub(super) fn fixture() -> (tempfile::TempDir, String) {
    fixture_with(&["init", "-q"])
}
fn fixture_with(init: &[&str]) -> (tempfile::TempDir, String) {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), init);
    for (key, value) in [
        ("user.name", "test"),
        ("user.email", "test@example.invalid"),
        ("core.autocrlf", "false"),
    ] {
        git(root.path(), &["config", key, value]);
    }
    write(root.path(), ".gitignore", b"ignored/\n");
    write(root.path(), "tracked", b"base");
    write(root.path(), "dir/file", b"base");
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-qm", "base"]);
    let base = git(root.path(), &["rev-parse", "HEAD"]);
    (root, base)
}
pub(super) fn snapshot(root: &Path, base: &str, paths: &[&str]) -> BaseSnapshot {
    let tree = RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: root.to_string_lossy().into(),
        base_commit: base.into(),
        decomposition_run_id: "fixture".into(),
        recorded_at: "fixture".into(),
    })
    .unwrap();
    snapshot_deliverables(&tree, root, paths.iter().copied()).unwrap()
}
fn failure(result: Result<BaseCopy, Unrun>) -> String {
    match result {
        Err(error) => {
            let cause = error.2.expect("base refusal must have a typed cause");
            assert!(match cause {
                UnrunCause::BaselineFetchFailed => error.0.starts_with("baseline_fetch_failed:"),
                UnrunCause::BaselineUnavailable => error.0.starts_with("baseline_unavailable:"),
                UnrunCause::SnapshotTampered => error.0.starts_with("snapshot_tampered:"),
            });
            error.0
        }
        Ok(_) => panic!("copy unexpectedly succeeded"),
    }
}
#[test]
fn base_copy_holds_only_base_and_no_refs_alternates_or_later_commit() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    write(source, "tracked", b"head");
    git(source, &["add", "."]);
    git(source, &["commit", "-qm", "head"]);
    let head = git(source, &["rev-parse", "HEAD"]);
    write(source, "tracked", b"live");
    write(source, "ignored/file", b"live");
    write(source, "ignored/dir/file", b"live");
    let snapshot = snapshot(
        source,
        &base,
        &[
            "tracked",
            "dir",
            "dir/file",
            "ignored/file",
            "ignored/dir",
            "absent",
        ],
    );
    let copy =
        materialise_base_copy(parent.path(), source, source, None, &snapshot, &base).unwrap();
    assert_eq!(
        std::fs::read(copy.repository().join("tracked")).unwrap(),
        b"base"
    );
    assert_eq!(std::fs::read(source.join("tracked")).unwrap(), b"live");
    for key in ["ignored/file", "ignored/dir", "absent"] {
        assert!(!copy.project().join(key).exists());
    }
    assert_eq!(copy.commit(), base);
    assert_eq!(git(copy.repository(), &["rev-parse", "HEAD"]), base);
    assert!(git(copy.repository(), &["for-each-ref"]).is_empty());
    let alternates = git(
        copy.repository(),
        &["rev-parse", "--git-path", "objects/info/alternates"],
    );
    assert!(!copy.repository().join(alternates).exists());
    assert!(
        !command(copy.repository(), &["cat-file", "-e", &head])
            .status
            .success()
    );
    let path = copy.layout().root;
    copy.remove().unwrap();
    assert!(!path.exists());
}
#[test]
fn untracked_project_keeps_sibling_inputs_and_excludes_all_deliverables() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    let project = source.join("untracked");
    write(&project, "input", b"input");
    write(&project, "output", b"deliverable");
    let snapshot = snapshot(source, &base, &["untracked/output"]);
    let copy =
        materialise_base_copy(parent.path(), source, &project, None, &snapshot, &base).unwrap();
    assert_eq!(
        std::fs::read(copy.project().join("input")).unwrap(),
        b"input"
    );
    assert!(!copy.project().join("output").exists());
    drop(copy);
    let snapshot = snapshot_deliverables(
        &RepositoryTree::load(&RepositoryRecordV1 {
            schema_version: 1,
            repository_root: source.to_string_lossy().into(),
            base_commit: base.clone(),
            decomposition_run_id: "fixture".into(),
            recorded_at: "fixture".into(),
        })
        .unwrap(),
        &project,
        ["untracked"],
    )
    .unwrap();
    let copy =
        materialise_base_copy(parent.path(), source, &project, None, &snapshot, &base).unwrap();
    assert!(std::fs::read_dir(copy.project()).unwrap().next().is_none());
}
#[test]
fn tampered_digest_commit_and_rewritten_tracked_entry_are_refused_by_name() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    let original = snapshot(source, &base, &["tracked"]);
    let mut changed = original.clone();
    changed.digest.push('x');
    let message = failure(materialise_base_copy(
        parent.path(),
        source,
        source,
        None,
        &changed,
        &base,
    ));
    assert!(message.contains("snapshot_tampered") && message.contains("tracked"));
    let message = failure(materialise_base_copy(
        parent.path(),
        source,
        source,
        None,
        &original,
        "different",
    ));
    assert!(message.contains("snapshot_tampered") && message.contains("tracked"));
    let mut changed = original;
    changed
        .entries
        .insert("tracked".into(), SnapshotEntry::Absent);
    changed.digest = changed.recomputed_digest();
    let message = failure(materialise_base_copy(
        parent.path(),
        source,
        source,
        None,
        &changed,
        &base,
    ));
    assert!(message.contains("tracked (absent)"), "{message}");
    assert!(std::fs::read_dir(parent.path()).unwrap().next().is_none());
}
#[test]
fn unknown_pruned_and_beyond_depth_baselines_are_unavailable() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    let mut evidence = snapshot(source, &base, &["tracked"]);
    evidence.commit = "1234567890123456789012345678901234567890".into();
    evidence.digest = evidence.recomputed_digest();
    assert!(
        failure(materialise_base_copy(
            parent.path(),
            source,
            source,
            None,
            &evidence,
            &evidence.commit
        ))
        .contains("baseline_unavailable")
    );
    let old = snapshot(source, &base, &["tracked"]);
    git(source, &["checkout", "--orphan", "replacement"]);
    write(source, "tracked", b"replacement");
    git(source, &["add", "."]);
    git(source, &["commit", "-qm", "replacement"]);
    let replacement = git(source, &["rev-parse", "HEAD"]);
    let listed = git(source, &["for-each-ref", "--format=%(refname)"]);
    for reference in listed.lines() {
        git(source, &["update-ref", "-d", reference]);
    }
    git(source, &["checkout", "--detach", &replacement]);
    git(source, &["reflog", "expire", "--expire=now", "--all"]);
    git(source, &["prune", "--expire=now"]);
    assert!(
        failure(materialise_base_copy(
            parent.path(),
            source,
            source,
            None,
            &old,
            &base
        ))
        .contains("baseline_unavailable")
    );
    let (source, base) = fixture();
    let source = source.path();
    let old = snapshot(source, &base, &["tracked"]);
    write(source, "tracked", b"later");
    git(source, &["add", "."]);
    git(source, &["commit", "-qm", "later"]);
    let shallow = parent.path().join("shallow");
    git(
        parent.path(),
        &[
            "clone",
            "--no-local",
            "--depth=1",
            source.to_str().unwrap(),
            shallow.to_str().unwrap(),
        ],
    );
    assert!(
        failure(materialise_base_copy(
            parent.path(),
            &shallow,
            &shallow,
            None,
            &old,
            &base
        ))
        .contains("baseline_unavailable")
    );
}
#[test]
fn excludes_normalize_backslashes_and_honor_repository_case_policy() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    let project = source.join("untracked");
    for key in ["DIR/File", "Other/FILE", "lower/path"] {
        write(&project, key, b"secret");
    }
    write(&project, "input", b"input");
    git(source, &["config", "core.ignorecase", "true"]);
    let snapshot = snapshot(
        source,
        &base,
        &[
            "UNTRACKED/dir/file",
            "untracked/OTHER/file",
            "untracked/LOWER/PATH",
        ],
    );
    let copy =
        materialise_base_copy(parent.path(), source, &project, None, &snapshot, &base).unwrap();
    for key in ["DIR/File", "Other/FILE", "lower/path"] {
        assert!(!copy.project().join(key).exists());
    }
    assert!(copy.project().join("input").exists());
    let dest = parent.path().join("walk");
    let mut remaining = u64::MAX;
    copy_data_scoped(
        &project,
        Path::new(""),
        &dest,
        source,
        None,
        &[PathBuf::from("DIR\\File")],
        &mut remaining,
    )
    .unwrap();
    assert!(!dest.join("DIR/File").exists());
    write(copy.project(), "DIR/File", b"poison");
    assert!(
        copy.copy
            .base_guard
            .as_ref()
            .unwrap()
            .tripwire(&copy.copy)
            .unwrap_err()
            .0
            .contains("UNTRACKED/dir/file")
    );
}

#[test]
fn tripwire_detects_project_poison() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    write(source, "ignored/file", b"live");
    let snapshot = snapshot(source, &base, &["ignored/file"]);
    let copy =
        materialise_base_copy(parent.path(), source, source, None, &snapshot, &base).unwrap();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"input","digest":"anchor"},"gap_policy":{},"acceptance":[],"supplementary":[]})).unwrap();
    let guard = copy.copy.base_guard.as_ref().unwrap();
    guard.before_layout(&contract).unwrap();
    write(copy.project(), "ignored/file", b"poison");
    assert!(
        guard
            .tripwire(&copy.copy)
            .unwrap_err()
            .0
            .contains("ignored/file (unverified)")
    );
}

#[test]
fn sibling_sources_equal_inside_and_holding_nontracked_keys_are_refused() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    let project = source.join("project");
    std::fs::create_dir_all(&project).unwrap();
    write(source, "ignored/child/file", b"live");
    for (key, operand) in [
        ("ignored", "../ignored"),
        ("ignored", "../ignored/child"),
        ("ignored/child/file", "../ignored"),
    ] {
        let snapshot = snapshot(source, &base, &[key]);
        let copy =
            materialise_base_copy(parent.path(), source, &project, None, &snapshot, &base).unwrap();
        let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
            "schema_version":1,"prd":{"path":"input","digest":"anchor"},"gap_policy":{},
            "acceptance":[{"id":"AC-1","criterion":"one","check":{"kind":"command","command":format!("verify {operand}"),"cwd":"project_root"},"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":"test"}}],"supplementary":[]
        })).unwrap();
        let error = copy
            .copy
            .base_guard
            .as_ref()
            .unwrap()
            .before_layout(&contract)
            .unwrap_err();
        assert!(
            error.0.contains(key) && error.0.contains("sibling source"),
            "{}",
            error.0
        );
    }
}

#[test]
fn empty_excludes_preserve_copy_bytes_and_input_digest() {
    let (source, base) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("live");
    write(&project, "input", b"input");
    let before = data_digest(&project, source, None).unwrap();
    assert_eq!(
        before,
        data_digest_scoped(&project, source, None, Path::new(""), &[]).unwrap()
    );
    let a = HermeticCopy::at(parent.path(), source, &project, None, &base).unwrap();
    let b = HermeticCopy::at_excluding(parent.path(), source, &project, None, &base, &[]).unwrap();
    assert_eq!(
        archon_workflow::acceptance_scratch::inventory(&a.project).unwrap(),
        archon_workflow::acceptance_scratch::inventory(&b.project).unwrap()
    );
    assert_eq!(data_digest(&project, source, None).unwrap(), before);
}

#[test]
fn gitlink_placeholders_and_absent_paths_stay_out_of_the_copy() {
    let (source, old) = fixture();
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    git(
        source,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{old},sub"),
        ],
    );
    write(source, "literal[*]", b"base");
    git(source, &["add", "literal[*]"]);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("tracked", source.join("link")).unwrap();
        git(source, &["add", "link"]);
    }
    git(source, &["commit", "-qm", "objects"]);
    let base = git(source, &["rev-parse", "HEAD"]);
    let keys = [
        "sub",
        "sub/inner",
        "absent",
        "absent-dir",
        "ignored/absent",
        "literal[*]",
    ];
    let evidence = snapshot(source, &base, &keys);
    let copy =
        materialise_base_copy(parent.path(), source, source, None, &evidence, &base).unwrap();
    for key in &keys[..5] {
        assert!(!copy.project().join(key).exists(), "{key}");
    }
    assert_eq!(
        std::fs::read(copy.project().join("literal[*]")).unwrap(),
        b"base"
    );
    #[cfg(unix)]
    {
        let evidence = snapshot(source, &base, &["link"]);
        let copy =
            materialise_base_copy(parent.path(), source, source, None, &evidence, &base).unwrap();
        assert_eq!(
            std::fs::read_link(copy.project().join("link")).unwrap(),
            Path::new("tracked")
        );
    }
}

#[test]
fn base_copy_preserves_the_repository_object_format() {
    let (source, base) = fixture_with(&["init", "-q", "--object-format=sha256"]);
    let source = source.path();
    let parent = tempfile::tempdir().unwrap();
    let evidence = snapshot(source, &base, &["tracked", "dir", "dir/file"]);
    let copy =
        materialise_base_copy(parent.path(), source, source, None, &evidence, &base).unwrap();
    assert_eq!(
        git(copy.repository(), &["rev-parse", "--show-object-format"]),
        git(source, &["rev-parse", "--show-object-format"])
    );
    assert_eq!(copy.commit(), base);
    assert_eq!(
        std::fs::read(copy.project().join("tracked")).unwrap(),
        b"base"
    );
}
