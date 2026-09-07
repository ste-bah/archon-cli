use super::*;

#[test]
fn native_policy_is_strict_and_opt_in() {
    let empty: archon_core::config::ArchonConfig = toml::from_str("").unwrap();
    assert!(empty.workflow.acceptance_execution.is_none());
    assert!(
        toml::from_str::<archon_core::config::ArchonConfig>(
            "[workflow.acceptance_execution]\nunknown_authority=true\n"
        )
        .is_err()
    );
}

#[test]
fn native_policy_is_captured_from_host_config_and_binds_recorded_repository() {
    let project = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("tasks")).unwrap();
    std::fs::create_dir_all(project.path().join(".archon")).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "user.name", "fixture"]);
    std::fs::write(repo.path().join("input"), "committed").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
    let scratch = tempfile::tempdir().unwrap();
    let config = format!(
        r#"[workflow.acceptance_execution]
repository={:?}
scratch_parent={:?}
project_inputs=[]
environment_allowlist=["FIXTURE_HOST_TOKEN"]
project_repository_view="combined"
toolchain_path="/usr/bin:/bin"
timeout_secs=10
output_bytes=4096
scratch_bytes=16777216
"#,
        repo.path().display().to_string(),
        scratch.path().display().to_string()
    );
    std::fs::write(project.path().join(".archon/config.toml"), config).unwrap();
    let binding = crate::command::acceptance_scratch_policy::capture(
        project.path(),
        &project.path().join("tasks"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        binding.policy.repository,
        repo.path().canonicalize().unwrap()
    );
    assert!(binding.policy.combined);
    assert_eq!(binding.policy.environment_allowlist, vec!["FIXTURE_HOST_TOKEN"]);
    assert!(binding.policy.environment.is_empty());
    assert_eq!(binding.source_commit.len(), 40);
    std::fs::write(
        project.path().join(".archon/config.toml"),
        "[workflow.acceptance_execution]\nunknown=true\n",
    )
    .unwrap();
    assert!(
        crate::command::acceptance_scratch_policy::capture(
            project.path(),
            &project.path().join("tasks")
        )
        .is_err()
    );
}

#[test]
fn native_final_source_records_implementation_commit_not_launch_commit() {
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(o.status.success());
        String::from_utf8(o.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    git(&["config", "user.name", "fixture"]);
    std::fs::write(repo.path().join("input"), "old").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "old"]);
    let old = git(&["rev-parse", "HEAD"]);
    std::fs::write(repo.path().join("input"), "implemented").unwrap();
    git(&["commit", "-qam", "implemented"]);
    let new = git(&["rev-parse", "HEAD"]);
    let store_root = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::project(store_root.path());
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "test".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    let binding = serde_json::json!({"source_commit":old,"policy":{"repository":repo.path()}});
    let captured =
        crate::command::acceptance_scratch_policy::record_final_source(&store, &run.id, &binding)
            .unwrap();
    assert_eq!(captured["source_commit"], new);
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(&run.id).join("observer/source-revision.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["commit"], new);
    // A later checkout change cannot rebind a previously recorded observation.
    git(&["checkout", "--detach", &old]);
    let captured =
        crate::command::acceptance_scratch_policy::record_final_source(&store, &run.id, &binding)
            .unwrap();
    assert_eq!(captured["source_commit"], new);
}
