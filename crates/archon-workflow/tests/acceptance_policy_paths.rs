//! Policy validation must use the host's PATH grammar, without enabling execution.
use archon_workflow::acceptance_scratch::ScratchPolicy;

fn policy() -> (tempfile::TempDir, ScratchPolicy) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let path = std::env::join_paths([root.join("tools"), root.join("more tools")]).unwrap();
    let policy = ScratchPolicy {
        repository: root.join("repo"),
        project: root.join("project"),
        task_root: root.join("project/tasks"),
        scratch_parent: root.join("scratch"),
        project_inputs: vec!["data".into()],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: path.into_string().unwrap(),
        environment: Default::default(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 10,
        output_bytes: 1024,
        scratch_bytes: 1024,
        build_cache: None,
    };
    (temp, policy)
}

#[test]
fn absolute_native_path_entries_validate_and_relative_or_empty_entries_do_not() {
    let (_temp, mut policy) = policy();
    policy.validate().expect("native absolute PATH entries");
    for entries in [
        vec!["relative".into()],
        vec![std::path::PathBuf::new()],
        vec![policy.repository.clone(), std::path::PathBuf::new()],
    ] {
        policy.toolchain_path = std::env::join_paths(entries)
            .unwrap()
            .into_string()
            .unwrap();
        assert!(policy.validate().is_err(), "{:?}", policy.toolchain_path);
    }
}
