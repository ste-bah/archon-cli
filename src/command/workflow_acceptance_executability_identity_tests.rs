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
fn same_size_project_rewrite_with_restored_mtime_changes_key() {
    let trees = trees(&[("AC-I-002", "true", TrustedCwd::RepoRoot)]);
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let path = trees.set.project.path().join("data/input.txt");
    std::fs::write(&path, "one").unwrap();
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let key = || {
        check_key(
            &HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks),
            &tree,
            &trees.contract(),
            "AC-I-002",
        )
        .unwrap()
    };
    let before = key();
    std::fs::write(&path, "two").unwrap();
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_ne!(before, key());
}

/// Issue 280: a probe keys its verdicts by the environment it was built
/// with. The process environment changing after that (a test beside it, a
/// variable set later) does not split its keys from those of a probe built
/// before the change.
#[test]
fn a_probe_built_before_an_environment_change_keys_as_before() {
    crate::test_env::run_alone!(a_probe_built_before_an_environment_change_keys_as_before);
    let trees = trees(&[("AC-I-003", "true", TrustedCwd::RepoRoot)]);
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let probe = || HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let key = |probe: &HostProbe| check_key(probe, &tree, &trees.contract(), "AC-I-003").unwrap();
    let saved = key(&probe());
    let retry = probe();
    // SAFETY: this test runs alone in its own process (`run_alone!`).
    unsafe { crate::test_env::set_var("ARCHON_ISSUE_280_PROBE", "set-beside-the-probe") };
    assert_eq!(key(&retry), saved);
}

/// Issue 280: the key reads the environment the probe was given, and only
/// that.
#[test]
fn a_probe_keys_by_the_environment_it_was_given() {
    let trees = trees(&[("AC-I-004", "true", TrustedCwd::RepoRoot)]);
    let tree = Baseline {
        repository: trees.repo.clone(),
        commit: git_head(&trees.repo).unwrap(),
    };
    let key = |environment: &BTreeMap<String, String>| {
        let probe = HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks)
            .with_host_environment(environment.clone());
        check_key(&probe, &tree, &trees.contract(), "AC-I-004").unwrap()
    };
    let mut environment = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
    let original = key(&environment);
    assert_eq!(key(&environment), original);
    environment.insert("LANG".into(), "C".into());
    assert_ne!(key(&environment), original);
}
