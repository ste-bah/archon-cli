use super::*;
#[test]
fn legacy_decisions_are_migrated_persisted_and_read_without_redeciding() {
    use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
    let root = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(root.path());
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            focused_tests: vec!["`check --one`".into()],
            required_tools: vec!["check".into()],
            ..Default::default()
        }],
    };
    let mut value = serde_json::json!({"task_universe": universe, "check_environment_policy": {"toolchain_path":root.path().to_string_lossy(), "bound":{}, "forwarded":[]}});
    assert!(migrate_focused_decisions(&mut value, Some(root.path())).unwrap());
    store
        .write_run_json("migration", GENERATED_V2_METADATA_PATH, &value)
        .unwrap();
    let bytes = fs::read(store.run_dir("migration").join(GENERATED_V2_METADATA_PATH)).unwrap();
    let mut persisted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    persisted["check_environment_policy"]["toolchain_path"] = serde_json::json!("unavailable");
    assert!(!migrate_focused_decisions(&mut persisted, Some(root.path())).unwrap());
    let migrated: WorkflowV2TaskUniverse =
        serde_json::from_value(persisted["task_universe"].clone()).unwrap();
    assert!(migrated.command_decisions_are_recorded());
    assert_eq!(
        migrated.tasks[0].declared_focused_test_commands(),
        ["check --one"]
    );
}

#[test]
fn a_captured_absent_search_path_does_not_read_current_configuration() {
    use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("config.toml"), "invalid = [").unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            focused_tests: vec!["`git diff --check`".into()],
            ..Default::default()
        }],
    };
    let mut value = serde_json::json!({"task_universe": universe,
        "check_environment_policy": {"toolchain_path":null, "bound":{}, "forwarded":[]}});
    assert!(migrate_focused_decisions(&mut value, Some(root.path())).unwrap());
    let migrated: WorkflowV2TaskUniverse =
        serde_json::from_value(value["task_universe"].clone()).unwrap();
    assert!(migrated.command_decisions_are_recorded());
    assert!(
        migrated.tasks[0]
            .declared_focused_test_commands()
            .is_empty()
    );
}

#[test]
fn a_legacy_default_site_is_captured_without_a_second_config_reader() {
    use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("config.toml"), "invalid = [").unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            focused_tests: vec!["`check --one`".into()],
            required_tools: vec!["check".into()],
            ..Default::default()
        }],
    };
    let mut value = serde_json::json!({"task_universe":universe});
    assert!(migrate_focused_decisions(&mut value, Some(root.path())).unwrap());
    let migrated: WorkflowV2TaskUniverse =
        serde_json::from_value(value["task_universe"].clone()).unwrap();
    assert_eq!(
        migrated.tasks[0].declared_focused_test_commands(),
        ["check --one"]
    );
    assert!(!migrate_focused_decisions(&mut value, Some(root.path())).unwrap());
}
