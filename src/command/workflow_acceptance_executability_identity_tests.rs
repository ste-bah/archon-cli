use super::super::probe_tests::trees;
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[test]
fn changed_binary_environment_or_toolchain_never_reuses_a_verdict_key() {
    let trees = trees(&[("AC-I-001", "test -f feature.txt", TrustedCwd::RepoRoot)]);
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let key = |identity| {
        let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
        probe.identity.set(identity).unwrap();
        check_key(&probe, &tree, &trees.contract(), "AC-I-001").unwrap()
    };
    let original =
        key(serde_json::json!(["revision", {"PATH":"/bin", "LANG":"C"}, ["rustc 1", "cargo 1"]]));
    for identity in [
        serde_json::json!(["other", {"PATH":"/bin", "LANG":"C"}, ["rustc 1", "cargo 1"]]),
        serde_json::json!(["revision", {"PATH":"/bin", "LANG":"en"}, ["rustc 1", "cargo 1"]]),
        serde_json::json!(["revision", {"PATH":"/bin", "LANG":"C"}, ["rustc 2", "cargo 1"]]),
    ] {
        assert_ne!(original, key(identity));
    }
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
