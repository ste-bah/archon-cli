//! A resume of a run whose acceptance contract and skeleton are frozen and
//! accepted, its acceptance freeze judged by an older freeze-acceptance
//! logic and its skeleton freeze by this build's: the resume verifies the
//! acceptance (verify-frozen-acceptance), replays the skeleton freeze
//! (freeze-skeleton, whose record holds), and re-freezes neither.
use super::*;

#[tokio::test]
async fn a_resume_of_an_accepted_frozen_chain_verifies_acceptance_and_replays_the_skeleton_freeze()
{
    let (project, store, run_id, state) = paused_frozen_run().await;
    let task_root = Path::new(&state.identity.task_root_identity);
    let current = crate::command::workflow_host_command_logic::versions();
    record_freeze(
        &store,
        &run_id,
        project.path(),
        task_root,
        "freeze-acceptance",
        "freeze-acceptance-1",
        "2026-10-10T10:00:00Z",
        "accepted",
        current["freeze-acceptance"] - 1,
        serde_json::json!([]),
        true,
    );
    record_freeze(
        &store,
        &run_id,
        project.path(),
        task_root,
        "freeze-skeleton",
        "freeze-skeleton-1",
        "2026-10-10T10:01:00Z",
        "accepted",
        current["freeze-skeleton"],
        serde_json::json!([]),
        true,
    );
    let dry_run = crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        &run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap();
    assert!(
        dry_run.contains("frozen_chain_override acceptance=verify"),
        "{dry_run}"
    );
    assert!(
        dry_run.contains("frozen_chain_override skeleton=keep reason=judged attempt freeze-skeleton-1 replays under this build's logic"),
        "{dry_run}"
    );
    assert!(
        dry_run.contains("capability=verify-frozen-acceptance"),
        "{dry_run}"
    );
    assert!(dry_run.contains("capability=freeze-skeleton"), "{dry_run}");
    assert!(
        !dry_run.contains("capability=freeze-acceptance"),
        "{dry_run}"
    );
    assert!(!dry_run.contains("acceptance-author"), "{dry_run}");
    assert!(
        !dry_run.contains("requirement_inventory_changed"),
        "{dry_run}"
    );

    let frozen = [
        archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE,
        archon_workflow::task_set_contract::TASK_SKELETON_FILE,
        archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE,
        archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE,
    ];
    let before = frozen.map(|name| std::fs::read(task_root.join(name)).unwrap());
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
        "accepted-chain-resume-test",
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    let script = crate::command::workflow_decompose_seed::resume::capture::take().unwrap();
    assert_eq!(script["frozenChain"]["acceptance"], true, "{script}");
    assert_eq!(script["frozenChain"]["skeleton"], false, "{script}");
    assert_eq!(
        before,
        frozen.map(|name| std::fs::read(task_root.join(name)).unwrap())
    );
    // The launch froze the requirement inventory; this resume derived the
    // same one, so it named no change.
    let inventory: serde_json::Value = read_json(
        &store
            .run_dir(&run_id)
            .join(crate::command::workflow_decompose_requirement_inventory::INVENTORY_PATH),
    );
    assert_eq!(inventory["recorded_at"], "launch", "{inventory}");
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(
        !events.contains("requirement_inventory_changed"),
        "{events}"
    );
}
