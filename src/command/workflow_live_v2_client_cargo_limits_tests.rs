use super::*;

#[cfg(unix)]
#[tokio::test]
async fn project_cargo_limits_reach_fixed_client_and_live_dispatch_command() {
    use archon_workflow::agent_dispatch_port::WorkflowAgentDispatch;

    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join(".archon")).unwrap();
    std::fs::write(
        project.path().join(".archon/config.toml"),
        "[tools.cargo]\nbuild_jobs = 2\nincremental = true\nresource_class = 'balanced'\n",
    )
    .unwrap();
    std::fs::write(project.path().join("Cargo.toml"), "[package]\n").unwrap();
    let config =
        archon_core::config_layers::load_layered_config(None, project.path(), None, None).unwrap();
    let limits = config.tools.cargo.to_limits();
    assert_eq!(limits.build_jobs, 2);
    assert!(limits.incremental);
    assert_eq!(limits.resource_class, "balanced");

    let (ui_sink, _ui) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = super::super::super::workflow_live_v2_fixed_run::fixed_decomposition_client(
        Arc::new(RecordingClient::default()),
        ui_sink,
        Vec::new(),
        "config-cargo-limits".into(),
        120,
        limits.clone(),
    );
    assert_eq!(client.cargo_limits(), Some(&limits));

    let dispatch =
        super::super::super::live_agent_dispatch::LiveAgentDispatch::new(client, limits.clone());
    let host = dispatch.host_command_env(project.path()).await;
    assert_eq!(host.resource_limits, limits);
    let mut environment = host.vars;
    archon_tools::workflow_resource_env::apply_workflow_resource_defaults(
        &mut environment,
        "cargo test",
        project.path(),
        &host.resource_limits,
    );
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s %s %s' \"$CARGO_BUILD_JOBS\" \"$CARGO_INCREMENTAL\" \"$ARCHON_WORKFLOW_RESOURCE_CLASS\"")
        .envs(environment)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "2 1 balanced");

    let observer_root = tempfile::tempdir().unwrap();
    let repository = observer_root.path().join("repo");
    let observer_project = observer_root.path().join("project");
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(observer_project.join("tasks")).unwrap();
    std::fs::create_dir_all(observer_project.join(".archon")).unwrap();
    std::fs::write(
        observer_project.join(".archon/config.toml"),
        "[tools.cargo]\nbuild_jobs = 2\nincremental = true\nresource_class = 'balanced'\n",
    )
    .unwrap();
    let observer_config =
        archon_core::config_layers::load_layered_config(None, &observer_project, None, None)
            .unwrap();
    let observer_limits = observer_config.tools.cargo.to_limits();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "test"]);
    std::fs::write(repository.join("Cargo.toml"), "[package]\nname='fixture'\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "fixture"]);
    let commit = git(&["rev-parse", "HEAD"]);
    let command = "test \"$CARGO_BUILD_JOBS\" = 2 && test \"$CARGO_INCREMENTAL\" = 1 && test \"$ARCHON_WORKFLOW_RESOURCE_CLASS\" = balanced";
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "prd": {"path":"p", "digest":"d"},
            "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
            "acceptance": [{
                "id":"AC-CARGO-001", "criterion":"configured cargo env",
                "check":{"kind":"command", "command":command, "cwd":"project_root"},
                "judgment":{"verdict":"accepted", "counterexample":"missing env", "reason":"checks env", "host_call_id":"test"}
            }]
        }))
        .unwrap();
    let references = [archon_workflow::acceptance_world::FrozenCommandRef {
        acceptance_id: "AC-CARGO-001".into(),
        kind: archon_workflow::acceptance_world::AcceptanceCommandKind::Command,
        chain_digest: "chain".into(),
        command_digest: archon_workflow::task_set_contract::content_digest(command.as_bytes()),
    }];
    let policy = archon_workflow::acceptance_scratch::ScratchPolicy {
        repository,
        project: observer_project.clone(),
        task_root: observer_project.join("tasks"),
        scratch_parent: observer_root.path().join("scratch"),
        project_inputs: vec![],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: "/usr/bin:/bin:/usr/sbin:/sbin".into(),
        environment: std::collections::BTreeMap::new(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 10,
        output_bytes: 2048,
        scratch_bytes: 16 * 1024 * 1024,
        build_cache: None,
    };
    let observed = archon_workflow::acceptance_scratch::observe_commands(
        &policy,
        &commit,
        &contract,
        "chain",
        &references,
        &observer_root.path().join("evidence"),
        observer_limits,
    )
    .await
    .unwrap();
    assert_eq!(
        observed.checks[0].exit_code,
        Some(0),
        "{:?}",
        observed.checks[0]
    );

    std::fs::write(
        project.path().join(".archon/config.toml"),
        "[tools.cargo\nbuild_jobs = 2",
    )
    .unwrap();
    assert!(
        archon_core::config_layers::load_layered_config(None, project.path(), None, None).is_err(),
        "a broken project config must fail resolution instead of authorizing built-in limits"
    );
}
