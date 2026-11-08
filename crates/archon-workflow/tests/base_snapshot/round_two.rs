use super::*;

#[test]
fn r18_snapshot_bench_uses_real_base_inventory_and_all_deliverables() {
    let repository = repo();
    let root = repository.path();
    let project = tempfile::tempdir().unwrap();
    for key in include_str!("../../../../tests/fixtures/r18_plan/base-paths.txt").lines() {
        write(root, key, b"base");
    }
    write(root, ".gitignore", b".archon/\n");
    git(root, &["add", "-f", "."]);
    git(root, &["commit", "-qm", "recorded base inventory"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    let skeleton: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/r18_plan/task-skeleton.json"
    ))
    .unwrap();
    let paths: std::collections::BTreeSet<_> = skeleton["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|task| task["deliverable_contracts"].as_array().unwrap())
        .map(|d| d["artifact_path"].as_str().unwrap())
        .collect();
    assert_eq!(paths.len(), 27);
    let tree = tree(root, &base);
    for key in &paths {
        if tree.exists_at_base(key) {
            continue;
        }
        let destination = if key.ends_with("evidence/citations.json")
            || key.ends_with("evidence/kb-rule-inventory.md")
        {
            root
        } else {
            project.path()
        };
        if key.ends_with("/backtests") {
            std::fs::create_dir_all(destination.join(key)).unwrap();
        } else {
            write(destination, key, b"live");
        }
    }
    let snapshot = snapshot_deliverables(&tree, project.path(), paths).unwrap();
    let mut counts = [0; 4];
    for entry in snapshot.entries.values() {
        counts[match entry {
            SnapshotEntry::Tracked { .. } => 0,
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::Ignored,
                ..
            } => 1,
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::OutsideRepository,
                live_digest: Some(_),
                ..
            } => 2,
            SnapshotEntry::Absent => 3,
            other => panic!("unexpected {other:?}"),
        }] += 1;
    }
    println!(
        "R18 tracked={} ignored={} outside_repository={} absent={}",
        counts[0], counts[1], counts[2], counts[3]
    );
    assert_eq!(counts, [19, 2, 6, 0]);
}

#[test]
fn project_only_evidence_is_classified_at_its_actual_location() {
    let repository = repo();
    let root = repository.path();
    write(root, ".gitignore", b"ignored/\ninside/ignored/\n");
    let base = commit(root);
    let external = tempfile::tempdir().unwrap();
    let inside = root.join("inside");
    for project in [external.path(), root, &inside] {
        for key in ["ignored/one", "ignored/two", "ignored/three"] {
            write(project, key, b"live");
        }
        let snapshot = snapshot_deliverables(
            &tree(root, &base),
            project,
            ["ignored/one", "ignored/two", "ignored/three"],
        )
        .unwrap();
        for entry in snapshot.entries.values() {
            let expected = if project == external.path() {
                UnverifiedReason::OutsideRepository
            } else {
                UnverifiedReason::Ignored
            };
            assert!(
                matches!(entry, SnapshotEntry::Unverified { reason, live_digest: Some(_), .. } if *reason == expected),
                "{entry:?}"
            );
        }
        if project == root {
            std::fs::remove_dir_all(root.join("ignored")).unwrap();
        }
    }
}

#[test]
fn absolute_keys_do_not_collide_and_unsafe_keys_are_individual() {
    let repository = repo();
    let root = repository.path();
    for key in ["etc/x", "etc/y", "etc/z"] {
        write(root, key, b"base");
    }
    let base = commit(root);
    let snapshot = snapshot_deliverables(
        &tree(root, &base),
        root,
        [
            "/etc/x",
            "etc/x",
            "/etc/y",
            "etc/y",
            "/etc/z",
            "etc/z",
            "",
            "../x",
            "a/../../x",
        ],
    )
    .unwrap();
    assert_eq!(snapshot.entries.len(), 9);
    for key in ["/etc/x", "/etc/y", "/etc/z", "", "../x", "a/../../x"] {
        assert!(matches!(
            snapshot.entries[key],
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::OutsideRepository,
                ..
            }
        ));
    }
    for key in ["etc/x", "etc/y", "etc/z"] {
        assert!(matches!(
            snapshot.entries[key],
            SnapshotEntry::Tracked { .. }
        ));
    }
}

#[test]
fn ignored_children_do_not_modify_tracked_directories() {
    let repository = repo();
    let root = repository.path();
    write(root, ".gitignore", b"ignored\n");
    for key in ["one/file", "two/file", "three/file"] {
        write(root, key, b"base");
    }
    let base = commit(root);
    for key in ["one/ignored", "two/ignored", "three/ignored"] {
        write(root, key, b"live");
    }
    let snapshot =
        snapshot_deliverables(&tree(root, &base), root, ["one", "two", "three"]).unwrap();
    for entry in snapshot.entries.values() {
        assert!(matches!(
            entry,
            SnapshotEntry::Tracked {
                live: LiveState::Same,
                ..
            }
        ));
    }
}

#[test]
fn empty_and_unborn_baselines_hold_nothing() {
    let repository = repo();
    let root = repository.path();
    // A blank base commit is a malformed record for the shared loader; the
    // classification helper maps an empty baseline to the unborn marker first.
    assert!(
        RepositoryTree::load(&RepositoryRecordV1 {
            schema_version: 1,
            repository_root: root.to_string_lossy().into(),
            base_commit: " ".into(),
            decomposition_run_id: "test".into(),
            recorded_at: "test".into(),
        })
        .is_err()
    );
    for base in ["unborn"] {
        write(root, "live", b"live");
        let snapshot =
            snapshot_deliverables(&tree(root, base), root, ["absent", "live", "other"]).unwrap();
        assert_eq!(snapshot.entries["absent"], SnapshotEntry::Absent);
        assert_eq!(snapshot.entries["other"], SnapshotEntry::Absent);
        assert!(matches!(
            snapshot.entries["live"],
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::Untracked,
                ..
            }
        ));
    }
    let base = commit(root);
    for base in ["unborn", base.as_str()] {
        let snapshot =
            snapshot_deliverables(&tree(root, base), root, ["live", "absent", "other"]).unwrap();
        assert_eq!(snapshot.entries["absent"], SnapshotEntry::Absent);
        if base == "unborn" {
            assert!(!matches!(
                snapshot.entries["live"],
                SnapshotEntry::Tracked { .. }
            ));
        } else {
            assert!(matches!(
                snapshot.entries["live"],
                SnapshotEntry::Tracked { .. }
            ));
        }
    }
}

#[test]
fn exclusion_failures_cover_more_and_case_mismatch_is_never_excluded() {
    let repository = repo();
    let root = repository.path();
    write(root, "tracked", b"base");
    let base = commit(root);
    for key in ["out/one", "out/two", "out/three"] {
        write(root, key, b"live");
    }
    let mut snapshot = snapshot_deliverables(
        &tree(root, &base),
        root,
        ["OUT/one", "OUT/two", "OUT/three"],
    )
    .unwrap();
    // Unset (git default false) and explicit false compare case-sensitively.
    let _ = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap(),
            "config",
            "--unset",
            "core.ignorecase",
        ])
        .status();
    for policy in [None, Some("false")] {
        if let Some(value) = policy {
            git(root, &["config", "core.ignorecase", value]);
        }
        let excluded = snapshot_excludes(root, root, true, &snapshot);
        for key in ["out/one", "out/two", "out/three"] {
            assert!(!excluded.contains(&key.into()), "{policy:?} {excluded:?}");
        }
    }
    // Invalid and explicit true policies must cover case variants.
    for policy in ["broken", "true"] {
        git(root, &["config", "core.ignorecase", policy]);
        let excluded = snapshot_excludes(root, root, true, &snapshot);
        for key in ["out/one", "out/two", "out/three"] {
            assert!(excluded.contains(&key.into()), "{policy} {excluded:?}");
        }
    }
    // A directory enumeration error (including not-a-directory) excludes its parent.
    for key in ["blocked-one", "blocked-two", "blocked-three"] {
        write(root, key, b"file");
    }
    let blocked = snapshot_deliverables(
        &tree(root, &base),
        root,
        [
            "blocked-one/child",
            "blocked-two/child",
            "blocked-three/child",
        ],
    )
    .unwrap();
    let excluded = snapshot_excludes(root, root, true, &blocked);
    for key in ["blocked-one", "blocked-two", "blocked-three"] {
        assert!(excluded.contains(&key.into()));
    }
    for entry in snapshot.entries.values_mut() {
        *entry = SnapshotEntry::Unverified {
            reason: UnverifiedReason::CaseMismatch,
            live_digest: None,
            live_note: None,
        };
    }
    for policy in ["true", "false", "broken"] {
        git(root, &["config", "core.ignorecase", policy]);
        assert!(snapshot_excludes(root, root, true, &snapshot).is_empty());
    }
}
