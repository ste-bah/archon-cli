use super::*;
use crate::command::test_support::absolute_toolchain_path;

#[tokio::test]
async fn fixed_resume_round_trips_both_recorded_check_policy_states() {
    for configured in [false, true] {
        let project = fixture_project();
        let mut config = launch_config(project.path());
        if configured {
            config.workflow.acceptance_execution =
                Some(archon_core::config::AcceptanceExecutionConfig {
                    repository: project.path().to_path_buf(),
                    scratch_parent: project.path().join("scratch"),
                    project_inputs: Vec::new(),
                    project_input_excludes: Vec::new(),
                    project_repository_view: Default::default(),
                    toolchain_path: absolute_toolchain_path(),
                    environment: Default::default(),
                    environment_allowlist: vec!["FIXTURE_API_KEY".into()],
                    cargo_seed: None,
                    timeout_secs: 60,
                    output_bytes: 1024,
                    scratch_bytes: 1024 * 1024,
                    external_data_roots: Vec::new(),
                });
        }
        let launch = BarrierFactory::launch(project.path().canonicalize().map(plain).unwrap());
        let _ = run_fixed_decomposition_with_factory(
            project.path(),
            Path::new("prds/PRD-X.md"),
            Path::new("tasks/PRD-X"),
            None,
            true,
            &config,
            &empty_env(),
            &launch,
        )
        .await;
        let store = WorkflowStore::project(project.path().canonicalize().map(plain).unwrap());
        let run = store.list_runs().unwrap().pop().unwrap();
        pause_run(&store, &run.id);
        let resume = BarrierFactory::resume(project.path().canonicalize().map(plain).unwrap());

        let error = resume_fixed_decomposition_with_factory(
            project.path(),
            &run.id,
            true,
            &config,
            &empty_env(),
            &resume,
        )
        .await
        .unwrap_err();

        assert!(
            format!("{error:#}").contains("barrier observed"),
            "{error:#}"
        );
        assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
        assert_eq!(store.list_runs().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn fixed_resume_refuses_missing_or_changed_check_policy_binding() {
    for tamper in ["missing", "null", "invalid"] {
        let project = fixture_project();
        let mut config = launch_config(project.path());
        config.workflow.acceptance_execution =
            Some(archon_core::config::AcceptanceExecutionConfig {
                repository: project.path().to_path_buf(),
                scratch_parent: project.path().join("scratch"),
                project_inputs: Vec::new(),
                project_input_excludes: Vec::new(),
                project_repository_view: Default::default(),
                toolchain_path: absolute_toolchain_path(),
                environment: Default::default(),
                environment_allowlist: vec!["FIXTURE_API_KEY".into()],
                cargo_seed: None,
                timeout_secs: 60,
                output_bytes: 1024,
                scratch_bytes: 1024 * 1024,
                external_data_roots: Vec::new(),
            });
        let launch = BarrierFactory::launch(project.path().canonicalize().map(plain).unwrap());
        let _ = run_fixed_decomposition_with_factory(
            project.path(),
            Path::new("prds/PRD-X.md"),
            Path::new("tasks/PRD-X"),
            None,
            true,
            &config,
            &empty_env(),
            &launch,
        )
        .await;
        let store = WorkflowStore::project(project.path().canonicalize().map(plain).unwrap());
        let run = store.list_runs().unwrap().pop().unwrap();
        pause_run(&store, &run.id);
        let path = store.run_dir(&run.id).join("v2/generated-metadata.json");
        let mut metadata: serde_json::Value = read_json(&path);
        match tamper {
            "missing" => {
                metadata
                    .as_object_mut()
                    .unwrap()
                    .remove("check_environment_policy");
            }
            "null" => metadata["check_environment_policy"] = serde_json::Value::Null,
            _ => metadata["check_environment_policy"] = serde_json::json!({ "extra": true }),
        }
        store
            .write_run_json(&run.id, "v2/generated-metadata.json", &metadata)
            .unwrap();
        let factory = PanicFactory {
            builds: AtomicUsize::new(0),
        };
        let error = resume_fixed_decomposition_with_factory(
            project.path(),
            &run.id,
            true,
            &config,
            &empty_env(),
            &factory,
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().contains("check policy")
                || error.to_string().contains("launch snapshot"),
            "{tamper}: {error:#}"
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    }
}
