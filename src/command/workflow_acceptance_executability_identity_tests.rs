use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

/// A key function for check `check` in one fixture: the key under a probe
/// whose runtime identity is `[binary revision, environment, toolchain]`.
fn keys_for(check: &str) -> impl Fn(serde_json::Value) -> String {
    let id = "AC-I-001";
    let trees = trees(&[(id, check, TrustedCwd::RepoRoot)]);
    move |identity| key_under(&trees, id, identity)
}

/// The key of check `id` of `trees` under a probe whose runtime identity is
/// `identity`.
fn key_under(
    trees: &super::super::probe_tests::Trees,
    id: &str,
    identity: serde_json::Value,
) -> String {
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    probe.identity.set(identity).unwrap();
    check_key(&probe, &tree, &trees.contract(), id).unwrap()
}

// Issue 288 (cross-build reuse): replaces
// `changed_binary_environment_or_toolchain_never_reuses_a_verdict_key`. Every
// environment and toolchain assertion is kept; a binary revision alone no
// longer re-keys a bounded check (see
// `unrelated_build_change_reuses_identical_bounded_verdict`).
#[test]
fn changed_environment_or_toolchain_never_reuses_a_verdict_key() {
    for check in ["test -f feature.txt", "cargo test"] {
        let key = keys_for(check);
        let original = key(
            serde_json::json!(["revision", {"PATH":"/bin", "LANG":"C"}, ["rustc 1", "cargo 1"]]),
        );
        for identity in [
            serde_json::json!(["revision", {"PATH":"/bin", "LANG":"en"}, ["rustc 1", "cargo 1"]]),
            serde_json::json!(["revision", {"PATH":"/bin", "LANG":"C"}, ["rustc 2", "cargo 1"]]),
        ] {
            assert_ne!(original, key(identity), "{check}");
        }
    }
}

/// Issue 288: a redeploy that changes no freeze-acceptance logic keeps the
/// verdict of a bounded literal check. Its key names the logic version and
/// pinned digest, not the binary that judged it.
#[test]
fn unrelated_build_change_reuses_identical_bounded_verdict() {
    let keys = keys_for("test -f feature.txt");
    let key = |revision: &str| {
        keys(serde_json::json!([revision, {"PATH":"/bin", "LANG":"C"}, ["rustc 1", "cargo 1"]]))
    };
    assert_eq!(key("revision"), key("other"));
}

/// Issue 288, operator decision 2026-10-14: only bounded literal checks
/// reuse a verdict across builds. Any other check still runs after a
/// redeploy.
#[test]
fn unbounded_check_still_executes_after_redeploy() {
    let keys = keys_for("cargo test");
    let key = |revision: &str| {
        keys(serde_json::json!([revision, {"PATH":"/bin", "LANG":"C"}, ["rustc 1", "cargo 1"]]))
    };
    assert_eq!(key("revision"), key("revision"));
    assert_ne!(key("revision"), key("other"));
}

/// Issue 288: the bounded key binds the freeze-acceptance logic. Another
/// version or another pinned source digest never reuses a verdict.
#[test]
fn freeze_logic_digest_change_invalidates_verdict() {
    let trees = trees(&[("AC-I-004", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    probe
        .identity
        .set(serde_json::json!(["revision", {"PATH":"/bin"}, ["rustc 1", "cargo 1"]]))
        .unwrap();
    let data = data_state(&probe, &tree).unwrap();
    let contract = trees.contract();
    let entry = contract
        .acceptance
        .iter()
        .find(|entry| entry.id == "AC-I-004")
        .unwrap();
    let key = |version: u32, digest: &str| {
        reuse::bounded_memo_key_for(&probe, &tree, &data, entry, (version, digest)).unwrap()
    };
    let original = key(7, "digest-a");
    assert_eq!(original, key(7, "digest-a"));
    assert_ne!(original, key(8, "digest-a"));
    assert_ne!(original, key(7, "digest-b"));
}

#[test]
fn changed_file_in_the_bounded_repository_closure_changes_key() {
    let trees = trees(&[("AC-I-002", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let commit = git_head(&trees.repo).unwrap();
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let before = check_key(
        &probe,
        &Baseline {
            repository: trees.repo.clone(),
            commit,
        },
        &trees.contract(),
        "AC-I-002",
    )
    .unwrap();

    std::fs::write(trees.repo.join("feature.txt"), "changed content").unwrap();
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(&trees.repo)
        .args(["add", "feature.txt"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(&trees.repo)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "changed fixture",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let after = check_key(
        &probe,
        &Baseline {
            repository: trees.repo.clone(),
            commit: git_head(&trees.repo).unwrap(),
        },
        &trees.contract(),
        "AC-I-002",
    )
    .unwrap();
    assert_ne!(before, after);
}

#[test]
fn unbounded_checks_keep_the_full_closure_reuse_key() {
    let trees = trees(&[("AC-I-003", "cargo test", TrustedCwd::RepoRoot)]);
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    assert!(check_key(&probe, &tree, &trees.contract(), "AC-I-003").is_some());
}

#[test]
fn real_check_keys_ignore_pipeline_and_harness_outputs_but_track_archon_project_data() {
    use archon_workflow::acceptance_scratch::ScratchPolicy;

    let trees = trees(&[("CHECK-KEY", "cargo test", TrustedCwd::RepoRoot)]);
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let task_root = trees.set.tasks.clone();
    let project = trees.set.project.path().to_path_buf();
    let data = project.join(".archon/project/data/check-input.json");
    std::fs::create_dir_all(data.parent().unwrap()).unwrap();
    std::fs::write(&data, b"project data v1").unwrap();

    let key = |scratch: bool| {
        let site = if scratch {
            let policy = ScratchPolicy {
                repository: trees.repo.clone(),
                project: project.clone(),
                task_root: task_root.clone(),
                scratch_parent: trees.outside.path().join("scratch-key-test"),
                project_inputs: vec![PathBuf::from(".archon")],
                project_input_excludes: Vec::new(),
                combined: false,
                toolchain_path: "/usr/bin:/bin".into(),
                environment: Default::default(),
                environment_allowlist: Vec::new(),
                cargo_seed: None,
                timeout_secs: 60,
                output_bytes: 8192,
                scratch_bytes: 16 << 20,
                build_cache: None,
            };
            super::Site::Scratch(Box::new(
                crate::command::acceptance_scratch_policy::NativeBinding {
                    policy,
                    source_commit: tree.commit.clone(),
                    external_data_roots: Vec::new(),
                },
            ))
        } else {
            super::Site::Hermetic
        };
        let mut probe = HostProbe::new(project.clone(), trees.repo.clone(), site);
        probe.task_root = Some(task_root.clone());
        check_key(&probe, &tree, &trees.contract(), "CHECK-KEY").unwrap()
    };

    for scratch in [false, true] {
        let before = key(scratch);
        std::fs::write(
            task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
            b"skeleton generated during decomposition",
        )
        .unwrap();
        std::fs::write(
            task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE),
            b"skeleton lock generated during decomposition",
        )
        .unwrap();
        assert_eq!(before, key(scratch), "skeleton outputs changed the key");
        std::fs::write(task_root.join("TASK-EXAMPLE-001.md"), b"landed task body").unwrap();
        assert_eq!(before, key(scratch), "task body changed the key");

        let shadow = project.join(".archon/logs/workflow-gates-shadow.jsonl");
        std::fs::create_dir_all(shadow.parent().unwrap()).unwrap();
        std::fs::write(&shadow, b"shadow record one\n").unwrap();
        assert_eq!(before, key(scratch), "shadow log changed the key");
        std::fs::write(&shadow, b"shadow record one\nshadow record two\n").unwrap();
        let learning = project.join(".archon/learning-state.db");
        std::fs::write(&learning, b"learning state one").unwrap();
        assert_eq!(before, key(scratch), "learning state changed the key");
        std::fs::write(&learning, b"learning state two").unwrap();
        assert_eq!(
            before,
            key(scratch),
            "changed learning state changed the key"
        );

        std::fs::write(&data, b"project data v2").unwrap();
        assert_ne!(before, key(scratch), "project data did not change the key");
        std::fs::write(&data, b"project data v1").unwrap();
        std::fs::remove_file(task_root.join("TASK-EXAMPLE-001.md")).unwrap();
        std::fs::remove_file(learning).unwrap();
        std::fs::remove_file(shadow).unwrap();
    }
}
