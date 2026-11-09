use super::*;

#[tokio::test]
async fn heal_unfrozen_preserves_prior_coverage_without_false_diagnostics() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let prd = format!(
        "{}\n- NFR-001: evidence\n",
        std::fs::read_to_string(&run.set.prd).unwrap()
    );
    std::fs::write(&run.set.prd, &prd).unwrap();
    let mut contract = run.set.contract();
    contract.prd.digest = content_digest(prd.as_bytes());
    contract.acceptance[0].covers = vec!["NFR-001".into()];
    write_chain(
        run.set.project.path(),
        &run.set.tasks,
        &contract,
        FreezeGateMode::Observe,
        0,
    );
    pre_implementation_head(&run);
    let launch = run.set.pin().identity();
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
            requirement_states: Default::default(),
            source_path: run.set.tasks.join("TASK-F-001.md").display().to_string(),
            ..Default::default()
        }],
    };
    crate::command::workflow_live::recover_bound_task_set(&run.store, Some(&universe)).unwrap();
    assert!(!run.set.pin_path().exists());
    assert_eq!(
        crate::command::workflow_task_set::recovery_lineage::refreeze_base(
            &run.set.pin_path(),
            &run.set.tasks,
        )
        .unwrap()
        .unwrap()
        .acceptance[0]
            .covers,
        ["NFR-001"]
    );
    let (_, record) = stage(&run, &accepting()).await;
    assert!(record.contract_repairs[0].repaired, "{record:?}");
    assert!(record.operational_errors.is_empty(), "{record:?}");
    assert_eq!(record.passed_check_ids(), ["AC-F-001"]);
    assert_eq!(run.set.contract().acceptance[0].covers, ["NFR-001"]);
    let log = std::fs::read_to_string(run.set.tasks.join(".decompose.log")).unwrap_or_default();
    assert!(!log.contains("uncovered item NFR-001"), "{log}");
}

#[tokio::test]
async fn diagnostics_write_failure_does_not_abort_the_round() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    let prd = format!(
        "{}\n- NFR-001: evidence\n",
        std::fs::read_to_string(&run.set.prd).unwrap()
    );
    std::fs::write(&run.set.prd, &prd).unwrap();
    let mut contract = run.set.contract();
    contract.prd.digest = content_digest(prd.as_bytes());
    write_chain(
        run.set.project.path(),
        &run.set.tasks,
        &contract,
        FreezeGateMode::Observe,
        0,
    );
    pre_implementation_head(&run);
    std::fs::create_dir(run.set.tasks.join(".decompose.log")).unwrap();
    let (_, record) = stage(&run, &accepting()).await;
    assert_eq!(record.passed_check_ids(), ["AC-F-001"]);
    assert!(record.operational_errors.is_empty(), "{record:?}");
}
