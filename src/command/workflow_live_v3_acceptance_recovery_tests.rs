//! Resumed acceptance after a recovery unfreeze, including repeat recovery.
use super::*;

/// An existing run retains its recorded launch anchor while recovery heals it.
#[tokio::test]
async fn a_resumed_run_heals_a_recovery_unfreeze_to_an_accepted_contract() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let launch = run.set.pin().identity();
    pre_implementation_head(&run);
    let metadata = run
        .store
        .run_dir(&run.run_id)
        .join("v2/generated-metadata.json");
    std::fs::create_dir_all(metadata.parent().unwrap()).unwrap();
    std::fs::write(&metadata, serde_json::json!({
        "schema_version": "test",
        "observer_snapshot": {
            "schema_version": 1,
            "canonical_task_root_identity": run.set.tasks.canonicalize().map(archon_shell::paths::plain).unwrap(),
            "expected_artifact_paths": [],
            "portable_acceptance_identity": launch,
            "lineage_recording": 1,
        }
    }).to_string()).unwrap();
    let lock = run.set.tasks.join(ACCEPTANCE_LOCK_FILE);
    let txn = "aabbccddeeff00112233445566778899";
    std::fs::write(
        lock.with_file_name(format!(".{ACCEPTANCE_LOCK_FILE}.{txn}.old")),
        b"legacy backup",
    )
    .unwrap();
    std::fs::write(
        &lock,
        b"an interrupted old-binary rollback left an invalid lock",
    )
    .unwrap();
    let universe = archon_workflow::task_universe::WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![run.set.tasks.display().to_string()],
        tasks: vec![archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
            source_path: run.set.tasks.join("TASK-F-001.md").display().to_string(),
            ..Default::default()
        }],
    };
    crate::command::workflow_live::recover_bound_task_set(&run.store, Some(&universe)).unwrap();
    assert!(!run.set.pin_path().exists());
    let (result, record) = stage(&run, &accepting()).await;
    assert!(
        record.operational_errors.is_empty(),
        "{:?}",
        record.operational_errors
    );
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(record.passed_check_ids(), ["AC-F-001"]);
    let pin = run.set.pin();
    assert!(
        !pin.lineage.is_empty(),
        "recovery re-freeze must have lineage"
    );
    let proof = crate::command::acceptance_chain::verify_launch_chain(
        &launch,
        archon_workflow::task_set_lineage::LaunchLineage::Recorded,
        &pin,
        &run.set.pin_path(),
        &run.set.tasks,
        &run.run_id,
    )
    .unwrap();
    assert!(!proof.identical);
    archon_workflow::task_skeleton::validate_full_chain(&run.set.tasks, &pin).unwrap();
    assert!(
        crate::command::acceptance_chain::verify_launch_chain(
            &launch,
            archon_workflow::task_set_lineage::LaunchLineage::Recorded,
            &pin,
            &run.set.pin_path(),
            &run.set.tasks,
            "a-run-not-recorded-by-recovery",
        )
        .is_err(),
        "the recovery receipt must authorize this run"
    );
    let mut stripped = pin.clone();
    stripped.lineage.clear();
    assert!(
        crate::command::acceptance_chain::verify_launch_chain(
            &launch,
            archon_workflow::task_set_lineage::LaunchLineage::Recorded,
            &stripped,
            &run.set.pin_path(),
            &run.set.tasks,
            &run.run_id,
        )
        .is_err(),
        "the receipt cannot replace pin lineage"
    );
    crate::command::acceptance_chain::verify_launch_chain(
        &pin.identity(),
        archon_workflow::task_set_lineage::LaunchLineage::Recorded,
        &pin,
        &run.set.pin_path(),
        &run.set.tasks,
        "a-later-run",
    )
    .unwrap();

    // Both the recovered run and a run launched after the recovery can
    // subsequently adopt an ordinary, recorded per-check re-author.
    let scope = crate::command::workflow_task_set::reauthor::AuthorScope::for_task_set(
        run.set.project.path(),
        &run.set.tasks,
        &run.set.prd,
    );
    let ids = ["AC-F-001".to_string()].into_iter().collect();
    let client = ScriptedAuthorJudge::new(
        |entry, _| {
            command_entry(
                entry,
                "test -f present && test -s present && test -r present",
            )
        },
        |_, _| true,
    );
    crate::command::workflow_task_set::republish::reauthor_and_republish(
        &client,
        crate::command::workflow_task_set::republish::ReauthorRequest {
            project_root: run.set.project.path(),
            tasks_root: &run.set.tasks,
            prd_path: &run.set.prd,
            ids: &ids,
            gate: run.set.gate(),
            trigger: "after recovery",
        },
        &scope,
    )
    .await
    .unwrap();
    for (anchor, id) in [
        (&launch, run.run_id.as_str()),
        (&pin.identity(), "a-later-run"),
    ] {
        crate::command::acceptance_chain::verify_launch_chain(
            anchor,
            archon_workflow::task_set_lineage::LaunchLineage::Recorded,
            &run.set.pin(),
            &run.set.pin_path(),
            &run.set.tasks,
            id,
        )
        .unwrap();
    }

    // Older recovery's moved-aside files remain for inspection. A second,
    // missing lock must record a NEW recovery event for this run.
    let lock = run.set.tasks.join(ACCEPTANCE_LOCK_FILE);
    std::fs::remove_file(&lock).unwrap();
    crate::command::workflow_live::recover_bound_task_set(&run.store, Some(&universe)).unwrap();
    let (again, record) = stage(&run, &accepting()).await;
    assert!(
        record.operational_errors.is_empty(),
        "repeat recovery: {:?}",
        record.operational_errors
    );
    assert_eq!(again.status, WorkflowV2Status::Accepted);
    archon_workflow::task_skeleton::validate_full_chain(&run.set.tasks, &run.set.pin()).unwrap();

    let refrozen = run.set.pin().identity();
    crate::command::workflow_task_set::republish::reauthor_and_republish(
        &client,
        crate::command::workflow_task_set::republish::ReauthorRequest {
            project_root: run.set.project.path(),
            tasks_root: &run.set.tasks,
            prd_path: &run.set.prd,
            ids: &ids,
            gate: run.set.gate(),
            trigger: "after repeat recovery",
        },
        &scope,
    )
    .await
    .unwrap();
    crate::command::acceptance_chain::verify_launch_chain(
        &refrozen,
        archon_workflow::task_set_lineage::LaunchLineage::Recorded,
        &run.set.pin(),
        &run.set.pin_path(),
        &run.set.tasks,
        "a-run-launched-after-repeat-recovery",
    )
    .unwrap();

    // Upgrade from a recovery that finished moving every artifact but did
    // not yet record recovery lineage: the retained pin still binds the data.
    let pin = run.set.pin_path();
    let sidecar = pin
        .parent()
        .unwrap()
        .join("check-sources")
        .join(pin.file_name().unwrap());
    for target in [
        run.set.tasks.join(ACCEPTANCE_LOCK_FILE),
        run.set.tasks.join(TASK_SKELETON_LOCK_FILE),
        pin,
        sidecar,
    ] {
        let name = target.file_name().unwrap().to_string_lossy();
        let aside = target.with_file_name(format!(
            ".{name}.unverified-99887766554433221100aabbccddeeff"
        ));
        std::fs::rename(target, aside).unwrap();
    }
    crate::command::workflow_live::recover_bound_task_set(&run.store, Some(&universe)).unwrap();
    let (fully_moved, record) = stage(&run, &accepting()).await;
    assert!(
        record.operational_errors.is_empty(),
        "completed legacy unfreeze: {:?}",
        record.operational_errors
    );
    assert_eq!(fully_moved.status, WorkflowV2Status::Accepted);
    archon_workflow::task_skeleton::validate_full_chain(&run.set.tasks, &run.set.pin()).unwrap();
}
