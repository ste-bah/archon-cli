use super::*;

#[test]
fn site_path_reaches_runtime_args_only_when_resolved() {
    let fixture = fixture();
    let arguments = serde_json::json!({
        "projectRoot": fixture.project, "taskRoot": fixture.tasks,
    });
    let path = format!("{}:/usr/bin:/bin", fixture.site_bin.display());
    let line = crate::command::acceptance_scratch_policy::check_site_path_line(&path);
    for known in [true, false] {
        if !known {
            std::fs::write(fixture.project.join(".archon/config.toml"), "[invalid").unwrap();
        }
        let runtime = crate::command::workflow_decompose::script_arguments_with_reply_markers(
            &arguments,
            &[],
        );
        assert_eq!(
            runtime["acceptanceCheckSitePathLine"].as_str(),
            known.then_some(line.as_str())
        );
        assert_eq!(
            crate::command::workflow_decompose::launch_metadata_arguments(&runtime),
            arguments
        );
    }
}

#[test]
fn fresh_task_root_gets_site_path_after_claim_creation() {
    let fixture = fixture();
    std::fs::remove_dir_all(&fixture.tasks).unwrap();
    assert!(!fixture.tasks.exists());
    let store = archon_workflow::WorkflowStore::project(&fixture.project);
    let state = archon_workflow::FixedDecompositionStateV1 {
        schema_version: archon_workflow::FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
        run_kind: archon_workflow::WorkflowRunKind::FixedDecompositionV1,
        identity: archon_workflow::FixedRunIdentityV1 {
            template_version: archon_workflow::FIXED_DECOMPOSITION_TEMPLATE_VERSION.into(),
            starting_binary_revision: "revision".into(),
            script_digest: "script".into(),
            catalog_digest: "catalog".into(),
            project_root_identity: fixture.project.display().to_string(),
            prd_identity: fixture.prd.display().to_string(),
            task_root_identity: fixture.tasks.display().to_string(),
        },
        phase: archon_workflow::DecompositionPhase::Identity,
        attempts: Default::default(),
        dispositions: Default::default(),
        log_path: fixture.tasks.join(".decompose.log").display().to_string(),
    };
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "fresh-site-path".into(),
        task: "claim fresh task root".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: vec![],
        permissions: Default::default(),
        learning_hooks: vec![],
    };
    crate::command::workflow_decompose::create_claimed_run(&store, &fixture.tasks, spec, &state)
        .unwrap();
    let arguments = serde_json::json!({
        "projectRoot": fixture.project, "taskRoot": fixture.tasks,
    });
    let runtime =
        crate::command::workflow_decompose::script_arguments_with_reply_markers(&arguments, &[]);
    let path = crate::command::acceptance_scratch_policy::check_site_path(
        &fixture.project,
        &fixture.tasks,
    )
    .unwrap();
    assert_eq!(
        runtime["acceptanceCheckSitePathLine"],
        crate::command::acceptance_scratch_policy::check_site_path_line(&path)
    );
}
