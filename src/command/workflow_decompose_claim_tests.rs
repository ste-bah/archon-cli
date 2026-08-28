use super::*;

fn fixed_state(task_root: &Path) -> FixedDecompositionStateV1 {
    FixedDecompositionStateV1 {
        schema_version: archon_workflow::FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
        run_kind: WorkflowRunKind::FixedDecompositionV1,
        identity: archon_workflow::FixedRunIdentityV1 {
            template_version: archon_workflow::FIXED_DECOMPOSITION_TEMPLATE_VERSION.into(),
            starting_binary_revision: "revision".into(),
            script_digest: "script".into(),
            catalog_digest: "catalog".into(),
            project_root_identity: task_root.parent().unwrap().display().to_string(),
            prd_identity: task_root
                .parent()
                .unwrap()
                .join("PRD.md")
                .display()
                .to_string(),
            task_root_identity: task_root.display().to_string(),
        },
        phase: archon_workflow::DecompositionPhase::Identity,
        attempts: Default::default(),
        dispositions: Default::default(),
        log_path: task_root.join(".decompose.log").display().to_string(),
    }
}

#[test]
fn concurrent_fixed_launch_claims_persist_exactly_one_run() {
    let temp = tempfile::tempdir().unwrap();
    let task_root = temp.path().join("tasks");
    std::fs::create_dir_all(&task_root).unwrap();
    let store = WorkflowStore::project(temp.path());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for ordinal in 0..2 {
        let store = store.clone();
        let task_root = task_root.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            let spec = archon_workflow::WorkflowSpec {
                schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
                name: format!("claim-{ordinal}"),
                task: "claim one task root".into(),
                target_repository_root: None,
                max_parallelism: 1,
                max_agents: 1,
                stages: Vec::new(),
                permissions: Default::default(),
                learning_hooks: Vec::new(),
            };
            let state = fixed_state(&task_root);
            barrier.wait();
            super::super::workflow_decompose::create_claimed_run(&store, &task_root, spec, &state)
        }));
    }
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();

    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);
    let runs = store.list_runs().unwrap();
    assert_eq!(runs.len(), 1);
    let persisted: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(
            store
                .run_dir(&runs[0].id)
                .join(FIXED_DECOMPOSITION_STATE_PATH),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        persisted.identity.task_root_identity,
        task_root.display().to_string()
    );

    let source = include_str!("workflow_decompose.rs");
    assert_eq!(source.matches("create_claimed_run(").count(), 2, "{source}");
}
