use super::tests::{fixture, git, materialise_base_copy, snapshot, write};
use super::*;

#[test]
fn inherited_filters_cannot_supply_deliverable_bytes() {
    if let Some(source) = std::env::var_os("S5_BASE_COPY_SOURCE") {
        let source = PathBuf::from(source);
        let base = std::env::var("S5_BASE_COPY_COMMIT").unwrap();
        let parent = tempfile::tempdir().unwrap();
        for key in ["one", "two", "three"] {
            assert_eq!(
                git(
                    &source,
                    &["cat-file", "--filters", &format!("{base}:{key}")]
                ),
                "poisoned"
            );
        }
        let evidence = snapshot(&source, &base, &["one", "two", "three"]);
        let copy =
            materialise_base_copy(parent.path(), &source, &source, None, &evidence, &base).unwrap();
        for key in ["one", "two", "three"] {
            assert_eq!(std::fs::read(copy.project().join(key)).unwrap(), b"base");
            assert_eq!(std::fs::read(source.join(key)).unwrap(), b"live");
        }
        return;
    }
    let (source, _) = fixture();
    let storage = tempfile::tempdir().unwrap();
    let driver = uuid::Uuid::new_v4().to_string();
    let mut attributes = String::new();
    for key in ["one", "two", "three"] {
        write(source.path(), key, b"base");
        attributes.push_str(&format!("{key} filter={driver}\n"));
    }
    write(source.path(), ".gitattributes", attributes.as_bytes());
    git(source.path(), &["add", "."]);
    git(source.path(), &["commit", "-qm", "filters"]);
    let base = git(source.path(), &["rev-parse", "HEAD"]);
    for key in ["one", "two", "three"] {
        write(source.path(), key, b"live");
    }
    let configuration = storage.path().join("configuration");
    std::fs::write(
        &configuration,
        format!("[filter \"{driver}\"]\nsmudge = echo poisoned\nrequired = true\n"),
    )
    .unwrap();
    let name = format!(
        "{}::inherited_filters_cannot_supply_deliverable_bytes",
        module_path!().split_once("::").unwrap().1
    );
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env("S5_BASE_COPY_SOURCE", source.path())
        .env("S5_BASE_COPY_COMMIT", base)
        .env("GIT_CONFIG_GLOBAL", configuration)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(String::from_utf8_lossy(&child.stdout).contains("1 passed; 0 failed"));
}

#[test]
fn case_mismatch_keys_keep_base_bytes_for_both_case_policies() {
    use archon_workflow::base_snapshot::UnverifiedReason;
    for policy in ["true", "false"] {
        let (source, _) = fixture();
        for key in ["out/one", "out/two", "out/three"] {
            write(source.path(), key, b"base");
        }
        git(source.path(), &["add", "."]);
        git(source.path(), &["commit", "-qm", "outputs"]);
        let base = git(source.path(), &["rev-parse", "HEAD"]);
        git(source.path(), &["config", "core.ignorecase", policy]);
        for key in ["out/one", "out/two", "out/three"] {
            write(source.path(), key, b"live");
        }
        let evidence = snapshot(
            source.path(),
            &base,
            &["Out/one", "OUT/two", "out/THREE", "tracked"],
        );
        for key in ["Out/one", "OUT/two", "out/THREE"] {
            assert!(matches!(
                evidence.entries[key],
                SnapshotEntry::Unverified {
                    reason: UnverifiedReason::CaseMismatch,
                    ..
                }
            ));
        }
        let parent = tempfile::tempdir().unwrap();
        let copy = materialise_base_copy(
            parent.path(),
            source.path(),
            source.path(),
            None,
            &evidence,
            &base,
        )
        .unwrap();
        for key in ["out/one", "out/two", "out/three", "tracked"] {
            assert_eq!(std::fs::read(copy.repository().join(key)).unwrap(), b"base");
        }
    }
}

#[test]
fn unsafe_and_outside_entries_do_not_refuse_other_deliverables() {
    let (source, base) = fixture();
    let parent = tempfile::tempdir().unwrap();
    let evidence = snapshot(
        source.path(),
        &base,
        &["", "../x", "/etc/x", "tracked/child", "tracked", "dir/file"],
    );
    let copy = materialise_base_copy(
        parent.path(),
        source.path(),
        source.path(),
        None,
        &evidence,
        &base,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(copy.repository().join("tracked")).unwrap(),
        b"base"
    );
    assert_eq!(
        std::fs::read(copy.repository().join("dir/file")).unwrap(),
        b"base"
    );
}

#[test]
fn baseline_availability_distinguishes_unknown_from_repository_failures() {
    for commit in [
        "unborn",
        "unknown",
        "1234567890123456789012345678901234567890",
    ] {
        let (source, base) = fixture();
        assert!(baseline_available(source.path(), &base).is_ok());
        let error = baseline_available(source.path(), commit).unwrap_err();
        assert_eq!(error.2, Some(UnrunCause::BaselineUnavailable), "{error:?}");
        let invalid = tempfile::tempdir().unwrap();
        let error = baseline_available(invalid.path(), commit).unwrap_err();
        assert_eq!(error.2, Some(UnrunCause::BaselineFetchFailed));
        assert!(error.0.contains("not a git repository"));
    }
}

#[test]
fn fetch_corrupt_objects_is_typed_separately_and_keeps_stderr() {
    for key in ["tracked", "dir/file", ".gitignore"] {
        let (source, base) = fixture();
        let object = git(source.path(), &["rev-parse", &format!("{base}:{key}")]);
        let object_path = source
            .path()
            .join(".git/objects")
            .join(&object[..2])
            .join(&object[2..]);
        // The commit still exists, but a real local transfer cannot read its content.
        let mut permissions = std::fs::metadata(&object_path).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(false);
        std::fs::set_permissions(&object_path, permissions).unwrap();
        std::fs::write(object_path, b"broken compressed object").unwrap();
        assert!(baseline_available(source.path(), &base).is_ok());
        let evidence = BaseSnapshot {
            landing_verified: Default::default(),
            commit: base.clone(),
            entries: Default::default(),
            digest: String::new(),
        };
        let mut evidence = evidence;
        evidence.digest = evidence.recomputed_digest();
        let parent = tempfile::tempdir().unwrap();
        let error = match materialise_base_copy(
            parent.path(),
            source.path(),
            source.path(),
            None,
            &evidence,
            &base,
        ) {
            Err(error) => error,
            Ok(_) => panic!("corrupt transfer unexpectedly succeeded"),
        };
        assert_eq!(error.2, Some(UnrunCause::BaselineFetchFailed), "{error:?}");
        assert!(
            error.0.contains("error:") || error.0.contains("fatal:"),
            "{error:?}"
        );
        assert!(std::fs::read_dir(parent.path()).unwrap().next().is_none());
    }
}

#[test]
fn subdirectory_copies_use_the_declared_relative_tree() {
    for subdirectory in ["one", "two/nested", "three/deep/nested"] {
        let (source, _) = fixture();
        write(source.path(), &format!("{subdirectory}/file"), b"base");
        git(source.path(), &["add", "."]);
        git(source.path(), &["commit", "-qm", "subdirectory"]);
        let base = git(source.path(), &["rev-parse", "HEAD"]);
        let selected = source.path().join(subdirectory);
        write(&selected, "file", b"live");
        let evidence = snapshot(&selected, &base, &["file", "missing", "tracked"]);
        assert!(matches!(
            evidence.entries["file"],
            SnapshotEntry::Tracked {
                live: archon_workflow::base_snapshot::LiveState::Modified,
                ..
            }
        ));
        assert_eq!(evidence.entries["tracked"], SnapshotEntry::Absent);
        let parent = tempfile::tempdir().unwrap();
        let copy =
            materialise_base_copy(parent.path(), &selected, &selected, None, &evidence, &base)
                .unwrap();
        assert_eq!(
            std::fs::read(copy.repository().join("file")).unwrap(),
            b"base"
        );
        assert!(!copy.repository().join("tracked").exists());
        assert!(!copy.repository().join("missing").exists());
    }
}
