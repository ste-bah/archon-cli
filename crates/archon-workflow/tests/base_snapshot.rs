use archon_workflow::base_snapshot::*;
use archon_workflow::repository_record::*;
use archon_workflow::task_set_contract::content_digest;
use std::path::Path;

#[path = "base_snapshot/classification.rs"]
mod classification;
use archon_workflow::v2::requirement_classes::RequirementClass;
fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().into()
}
fn repo() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.name", "test"]);
    git(
        root.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(root.path(), &["config", "core.autocrlf", "false"]);
    root
}
fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
fn commit(root: &Path) -> String {
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "fixture"]);
    git(root, &["rev-parse", "HEAD"])
}
fn tree(root: &Path, base: &str) -> RepositoryTree {
    RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: root.to_string_lossy().into(),
        base_commit: base.into(),
        decomposition_run_id: "test".into(),
        recorded_at: "test".into(),
    })
    .unwrap()
}
#[test]
fn tracked_live_states_nested_keys_and_literal_objects() {
    let root = repo();
    let root = root.path();
    for key in ["same", "modified", "deleted", "dir/file", "literal[*]"] {
        write(root, key, b"base");
    }
    let base = commit(root);
    write(root, "modified", b"head");
    commit(root);
    write(root, "modified", b"live");
    std::fs::remove_file(root.join("deleted")).unwrap();
    write(root, "dir/untracked", b"new");
    let snapshot = snapshot_deliverables(
        &tree(root, &base),
        root,
        [
            "same",
            "modified",
            "deleted",
            "dir",
            "dir/file",
            "literal[*]",
        ],
    )
    .unwrap();
    for (key, live) in [
        ("same", LiveState::Same),
        ("modified", LiveState::Modified),
        ("deleted", LiveState::Missing),
        ("dir", LiveState::Modified),
        ("dir/file", LiveState::Same),
        ("literal[*]", LiveState::Same),
    ] {
        let SnapshotEntry::Tracked {
            object,
            live: actual,
            ..
        } = &snapshot.entries[key]
        else {
            panic!("{snapshot:?}");
        };
        assert_eq!(*actual, live, "{key}");
        assert_eq!(*object, git(root, &["rev-parse", &format!("{base}:{key}")]));
    }
    assert!(snapshot.valid_digest());
}
#[test]
fn ignored_absent_added_and_normalized_keys() {
    let root = repo();
    let root = root.path();
    write(root, ".gitignore", b"ignored/\n");
    write(root, "src/x", b"base");
    let base = commit(root);
    write(root, "ignored/file", b"live");
    write(root, "ignored/dir/file", b"child");
    write(root, "new", b"new");
    git(root, &["add", "new"]);
    write(root, "untracked", b"data");
    let snapshot = snapshot_deliverables(
        &tree(root, &base),
        root,
        [
            "ignored/file",
            "ignored/dir",
            "absent",
            "missing-dir",
            "ignored/missing",
            "new",
            "untracked",
            "src\\x",
            "./src//x",
            "..\\y",
        ],
    )
    .unwrap();
    for key in ["ignored/file", "ignored/dir"] {
        assert!(matches!(
            &snapshot.entries[key],
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::Ignored,
                live_digest: Some(_),
                ..
            }
        ));
    }
    for key in ["absent", "missing-dir", "ignored/missing"] {
        assert_eq!(snapshot.entries[key], SnapshotEntry::Absent);
    }
    assert!(matches!(
        snapshot.entries["new"],
        SnapshotEntry::Unverified {
            reason: UnverifiedReason::AddedAfterBase,
            ..
        }
    ));
    assert!(matches!(
        snapshot.entries["untracked"],
        SnapshotEntry::Unverified {
            reason: UnverifiedReason::Untracked,
            ..
        }
    ));
    assert!(matches!(
        snapshot.entries["../y"],
        SnapshotEntry::Unverified {
            reason: UnverifiedReason::OutsideRepository,
            ..
        }
    ));
    assert_eq!(snapshot.entries.len(), 9);
    classification::assert_class(
        &snapshot,
        &["src/x", "ignored/file"],
        RequirementClass::Missing,
        Some("deliverable_unverified_at_base"),
    );
    classification::assert_class(
        &snapshot,
        &["absent", "missing-dir", "ignored/missing"],
        RequirementClass::Missing,
        Some("deliverables_absent_at_base"),
    );
    assert_eq!(
        live_digest(&root.join("ignored/file")),
        (Some(content_digest(b"live")), None)
    );
    let inventory =
        archon_workflow::acceptance_scratch::inventory(&root.join("ignored/dir")).unwrap();
    let lines = inventory
        .iter()
        .map(|(k, v)| format!("{k}\t{v}\n"))
        .collect::<String>();
    assert_eq!(
        live_digest(&root.join("ignored/dir")).0,
        Some(content_digest(lines.as_bytes()))
    );
    #[cfg(windows)]
    {
        let path = root.join("src/x").to_string_lossy().into_owned();
        let snapshot = snapshot_deliverables(&tree(root, &base), root, [path]).unwrap();
        assert!(snapshot.entries.contains_key("src/x"));
    }
}
#[test]
fn submodule_and_non_tree_ancestors_do_not_poison_ignore_query() {
    let root = repo();
    let root = root.path();
    write(root, ".gitignore", b"ignored/\n");
    write(root, "blob", b"base");
    #[cfg(unix)]
    std::os::unix::fs::symlink("blob", root.join("link")).unwrap();
    let base = commit(root);
    git(
        root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{base},sub"),
        ],
    );
    git(root, &["commit", "-qm", "link"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    write(root, "ignored/file", b"live");
    let paths = vec!["sub", "sub/inner", "blob/inner", "ignored/file"];
    #[cfg(unix)]
    let paths: Vec<_> = paths.into_iter().chain(["link", "link/inner"]).collect();
    let snapshot = snapshot_deliverables(&tree(root, &base), root, paths).unwrap();
    for key in ["sub", "sub/inner"] {
        assert!(matches!(
            snapshot.entries[key],
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::Submodule,
                ..
            }
        ));
    }
    assert!(matches!(
        snapshot.entries["blob/inner"],
        SnapshotEntry::Unverified {
            reason: UnverifiedReason::OutsideRepository,
            ..
        }
    ));
    assert!(matches!(
        snapshot.entries["ignored/file"],
        SnapshotEntry::Unverified {
            reason: UnverifiedReason::Ignored,
            ..
        }
    ));
    #[cfg(unix)]
    {
        assert!(matches!(
            snapshot.entries["link"],
            SnapshotEntry::Tracked {
                kind: SnapshotKind::Symlink,
                ..
            }
        ));
        assert!(matches!(
            snapshot.entries["link/inner"],
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::OutsideRepository,
                ..
            }
        ));
        std::os::unix::fs::symlink("ignored", root.join("live-link")).unwrap();
        let snapshot = snapshot_deliverables(&tree(root, &base), root, ["live-link/file"]).unwrap();
        assert!(matches!(
            snapshot.entries["live-link/file"],
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::OutsideRepository,
                ..
            }
        ));
    }
}
#[test]
fn filter_errors_keep_tracked_evidence_and_case_mismatch_is_unverified() {
    let root = repo();
    let root = root.path();
    for key in ["one", "two", "three"] {
        write(root, key, b"base");
    }
    let base = commit(root);
    write(root, ".gitattributes", b"* filter=broken\n");
    git(root, &["config", "filter.broken.required", "true"]);
    git(root, &["config", "filter.broken.clean", "exit 1"]);
    for key in ["one", "two", "three"] {
        write(root, key, b"live");
    }
    let snapshot =
        snapshot_deliverables(&tree(root, &base), root, ["one", "two", "three", "ONE"]).unwrap();
    for key in ["one", "two", "three"] {
        assert!(
            matches!(
                &snapshot.entries[key],
                SnapshotEntry::Tracked {
                    live: LiveState::Unknown,
                    live_note: Some(_),
                    ..
                }
            ),
            "{snapshot:?}"
        );
    }
    classification::assert_class(
        &snapshot,
        &["one", "two", "three"],
        RequirementClass::CandidateDone,
        None,
    );
    assert!(matches!(
        snapshot.entries["ONE"],
        SnapshotEntry::Unverified {
            reason: UnverifiedReason::CaseMismatch,
            ..
        }
    ));
}
#[test]
fn digest_limits_errors_and_legacy_classification() {
    let root = tempfile::tempdir().unwrap();
    for key in ["one", "two", "three"] {
        let path = root.path().join(key);
        let file = std::fs::File::create(&path).unwrap();
        file.set_len((1 << 30) + 1).unwrap();
        let (digest, note) = live_digest(&path);
        assert!(digest.is_none());
        assert!(note.unwrap().contains("exceeds 1 GiB"));
    }
    let (digest, note) = live_digest(&root.path().join("missing"));
    assert!(digest.is_none() && note.unwrap().contains("missing"));
    let legacy = serde_json::json!({"schema_version":1,"baseline_commit":"b","acceptance_digest":"a","baseline_results_digest":"r","error":null,"entries":[]});
    let record: archon_workflow::v2::requirement_classes::RequirementClassification =
        serde_json::from_value(legacy).unwrap();
    assert!(record.base_snapshot.is_none());
}

#[path = "base_snapshot/round_two.rs"]
mod round_two;

#[cfg(unix)]
#[test]
fn scratch_excludes_ignored_deliverables_and_keeps_sibling_input() {
    use archon_workflow::acceptance_scratch::{ScratchPolicy, ScratchRoots};
    let root = repo();
    let repository = root.path().canonicalize().unwrap();
    write(&repository, ".gitignore", b"ignored/\n");
    write(&repository, "tracked", b"base");
    let base = commit(&repository);
    for key in [
        "ignored/one",
        "ignored/two",
        "ignored/three",
        "ignored/input",
    ] {
        write(&repository, key, b"live");
    }
    let snapshot = snapshot_deliverables(
        &tree(&repository, &base),
        &repository,
        ["ignored/one", "ignored/two", "ignored/three"],
    )
    .unwrap();
    let parent = tempfile::tempdir().unwrap();
    let mut policy = ScratchPolicy {
        repository: repository.clone(),
        project: repository.clone(),
        task_root: repository.join("tasks"),
        scratch_parent: parent.path().join("copies"),
        project_inputs: vec!["ignored".into()],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: std::env::var("PATH").unwrap(),
        environment: Default::default(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 30,
        output_bytes: 4096,
        scratch_bytes: 16 << 20,
        build_cache: None,
    };
    std::fs::create_dir_all(&policy.task_root).unwrap();
    policy
        .project_input_excludes
        .extend(base_snapshot_excludes(&policy, &snapshot));
    let mut roots = ScratchRoots::prepare(&policy, &base).unwrap();
    for key in ["ignored/one", "ignored/two", "ignored/three"] {
        assert!(!roots.project().join(key).exists());
    }
    assert_eq!(
        std::fs::read(roots.project().join("ignored/input")).unwrap(),
        b"live"
    );
    roots.cleanup().unwrap();
}

#[test]
fn failed_ignore_query_is_unknown_for_each_live_key() {
    let root = repo();
    let root = root.path();
    write(root, "tracked", b"base");
    let base = commit(root);
    for key in ["one", "two", "three"] {
        write(root, key, b"live");
    }
    let excludes = root.join("exclude-directory");
    std::fs::create_dir(&excludes).unwrap();
    git(
        root,
        &["config", "core.excludesfile", excludes.to_str().unwrap()],
    );
    let snapshot =
        snapshot_deliverables(&tree(root, &base), root, ["one", "two", "three"]).unwrap();
    for key in ["one", "two", "three"] {
        let SnapshotEntry::Unverified {
            reason,
            live_note,
            live_digest,
        } = &snapshot.entries[key]
        else {
            panic!("{snapshot:?}");
        };
        assert_eq!(*reason, UnverifiedReason::IgnoreStatusUnknown);
        assert!(live_note.as_deref().unwrap().contains("exclude"));
        assert_eq!(
            live_digest.as_deref(),
            Some(content_digest(b"live").as_str())
        );
    }
}
