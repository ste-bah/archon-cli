use super::*;

#[test]
fn an_early_resume_needs_no_task_set_or_reverify_files() {
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "early-resume-test".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let run_dir = store.run_dir(&run.id);
    let task_root = temp.path().join("tasks-not-created-yet");
    let runtime = archon_workflow::FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: "revision".into(),
        script_digest: "script".into(),
        catalog_digest: "catalog".into(),
        project_root_identity: temp.path().display().to_string(),
        prd_identity: temp.path().join("PRD.md").display().to_string(),
        task_root_identity: task_root.display().to_string(),
    };
    let mut arguments = serde_json::json!({});
    let plan = prepare(
        &store,
        &run.id,
        &run_dir,
        &mut arguments,
        &runtime,
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert!(plan.is_none());
    assert_eq!(arguments["forceBodySubjects"], serde_json::json!([]));
    assert!(
        !task_root
            .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE)
            .exists()
    );
    assert!(
        !task_root
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE)
            .exists()
    );
}
