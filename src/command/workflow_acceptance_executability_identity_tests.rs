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
