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

    // One claim site in the launcher, one definition beside it: no second
    // path may create a fixed run without claiming its task root.
    let launcher = include_str!("workflow_decompose.rs");
    assert_eq!(
        launcher.matches("create_claimed_run(").count(),
        1,
        "{launcher}"
    );
    let claim = include_str!("workflow_decompose_claim.rs");
    assert_eq!(
        claim.matches("fn create_claimed_run(").count(),
        1,
        "{claim}"
    );
    assert_eq!(claim.matches("create_claimed_run(").count(), 1, "{claim}");
}

fn reclaim_fixture() -> (
    tempfile::TempDir,
    WorkflowStore,
    archon_workflow::WorkflowRun,
    FixedDecompositionStateV1,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let tasks = root.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let store = WorkflowStore::project(&root);
    let state = fixed_state(&tasks);
    let spec = archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "reclaim-test".into(),
        task: "reclaim stale root".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: vec![],
        permissions: Default::default(),
        learning_hooks: vec![],
    };
    let run =
        super::super::workflow_decompose::create_claimed_run(&store, &tasks, spec, &state).unwrap();
    std::fs::write(
        store.run_dir(&run.id).join("preserved-evidence.txt"),
        "evidence",
    )
    .unwrap();
    (temp, store, run, state)
}

#[test]
fn reclaimed_cancelled_owner_releases_root_without_deleting_evidence() {
    let (_temp, store, mut run, state) = reclaim_fixture();
    run.status = RunStatus::Cancelled;
    store.save_state(&run).unwrap();
    let generation = run.generation;
    let root = Path::new(&state.identity.task_root_identity);
    assert!(
        super::super::workflow_decompose::create_claimed_run(
            &store,
            root,
            run.spec.clone(),
            &state
        )
        .is_err()
    );
    // Inject only the liveness observation; exercise the real reclaim mutation.
    super::super::workflow_task_root_reclaim::reclaim_with_liveness(&store, &run.id, true, || {
        Ok(())
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(store.run_dir(&run.id).join("preserved-evidence.txt")).unwrap(),
        "evidence"
    );
    assert!(store.load_state(&run.id).unwrap().generation > generation);
    assert!(
        super::super::workflow_task_root_reclaim::require_not_reclaimed(&store, &run.id).is_err()
    );
    super::super::workflow_decompose::create_claimed_run(&store, root, run.spec, &state).unwrap();
}

/// A run that failed (the set gate stopped it, say) holds nothing: its root
/// can be entered again by a fresh launch without reclaim-task-root, which is
/// what a frozen-chain resume over a dead run needs. Cancelled and paused
/// runs stay resumable and keep their claim until reclaimed.
#[test]
fn a_failed_run_does_not_block_a_fresh_launch_on_its_task_root() {
    let (_temp, store, mut run, state) = reclaim_fixture();
    run.status = RunStatus::Failed;
    store.save_state(&run).unwrap();
    let root = Path::new(&state.identity.task_root_identity);
    assert!(!super::super::workflow_task_root_reclaim::is_reclaimed(&store, &run.id).unwrap());
    super::super::workflow_decompose::create_claimed_run(&store, root, run.spec.clone(), &state)
        .unwrap();
}

#[test]
fn reclaim_refuses_live_owner_and_missing_confirmation_without_mutation() {
    let (_temp, store, run, _state) = reclaim_fixture();
    let before = serde_json::to_vec(&store.load_state(&run.id).unwrap()).unwrap();
    assert!(
        super::super::workflow_task_root_reclaim::reclaim_with_liveness(
            &store,
            &run.id,
            false,
            || panic!("confirmation must precede mutation"),
        )
        .is_err()
    );
    assert!(
        super::super::workflow_task_root_reclaim::reclaim_with_liveness(
            &store,
            &run.id,
            true,
            || Err(anyhow::anyhow!("owner is live")),
        )
        .is_err()
    );
    assert_eq!(
        serde_json::to_vec(&store.load_state(&run.id).unwrap()).unwrap(),
        before
    );
}

#[test]
fn reclaim_refuses_held_execution_lease_then_allows_release() {
    let (_temp, store, run, _state) = reclaim_fixture();
    let lease = super::super::workflow_task_root_reclaim::begin_execution(&store, &run.id).unwrap();
    let error = super::super::workflow_task_root_reclaim::reclaim_with_liveness(
        &store,
        &run.id,
        true,
        || Ok(()),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("lock cannot be acquired"),
        "{error:#}"
    );
    drop(lease);
    super::super::workflow_task_root_reclaim::reclaim_with_liveness(&store, &run.id, true, || {
        Ok(())
    })
    .unwrap();
    assert!(super::super::workflow_task_root_reclaim::begin_execution(&store, &run.id).is_err());
}

#[tokio::test]
async fn reclaimed_run_cannot_resume_through_actual_entry_point() {
    let (_temp, store, run, state) = reclaim_fixture();
    super::super::workflow_task_root_reclaim::reclaim_with_liveness(&store, &run.id, true, || {
        Ok(())
    })
    .unwrap();
    let factory = PanicFactory {
        builds: AtomicUsize::new(0),
    };
    let error = super::super::workflow_decompose::resume_fixed_decomposition_with_factory(
        Path::new(&state.identity.project_root_identity),
        &run.id,
        true,
        &ArchonConfig::default(),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("reclaimed"), "{error:#}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
}
