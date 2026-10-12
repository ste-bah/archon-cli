use super::*;

#[tokio::test]
async fn revision_only_resume_persists_live_frozen_chain_without_a_phase_seed() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let task_root = Path::new(&state.identity.task_root_identity);
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_chain(
        project.path(),
        Path::new(&state.identity.prd_identity),
        task_root,
        &["TASK-X-010"],
    );
    let contract = archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    let skeleton = archon_workflow::task_set_contract::TASK_SKELETON_FILE;
    let acceptance_lock = archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE;
    let skeleton_lock = archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE;
    let before = [contract, skeleton, acceptance_lock, skeleton_lock]
        .map(|name| std::fs::read(task_root.join(name)).unwrap());
    let resume = UpgradeBarrier {
        run_id: run_id.clone(),
        store: store.clone(),
        expected: state.identity.clone(),
        builds: AtomicUsize::new(0),
    };
    let error = resume_fixed_decomposition_at_binary_revision(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
        "logic-version-only-revision",
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);

    let generation = store.load_state(&run_id).unwrap().generation;
    let record: serde_json::Value = read_json(&store.run_dir(&run_id).join(format!(
        "decomposition/resume-frozen-chain/generation-{generation}.json"
    )));
    assert_eq!(record["snapshot"]["acceptance"], true);
    assert_eq!(record["snapshot"]["skeleton"], true);
    assert!(
        !store
            .run_dir(&run_id)
            .join(crate::command::workflow_decompose_seed::SEEDS_DIR)
            .exists(),
        "revision-only drift must not need a phase seed"
    );
    assert_eq!(
        before,
        [contract, skeleton, acceptance_lock, skeleton_lock]
            .map(|name| std::fs::read(task_root.join(name)).unwrap())
    );
}
