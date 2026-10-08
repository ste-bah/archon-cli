use super::*;

fn restart_request(run_id: &str, action: &str, stage_id: &str, item_id: Option<&str>) -> WorkflowControlRequest {
    WorkflowControlRequest {
        run_id: run_id.into(),
        action: action.into(),
        stage_id: Some(stage_id.into()),
        item_id: item_id.map(str::to_string),
        rationale: None,
        confirmation_token: None,
    }
}

fn restart_fixture() -> (tempfile::TempDir, archon_workflow::WorkflowStore, archon_workflow::WorkflowRun) {
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::new(temp.path().join("runs"));
    let spec = archon_workflow::WorkflowSpec::from_yaml(
        "schema: archon.workflow.v1\nname: test\ntask: test\nstages:\n  - id: call-a\n    kind: agent\n",
    ).unwrap();
    let mut run = store.create_run(spec).unwrap();
    run.items.insert(
        "item-a".to_string(),
        archon_workflow::run::ItemState {
            id: "item-a".to_string(),
            stage_id: "call-a".to_string(),
            status: archon_workflow::StageStatus::Pending,
            artifact: None,
            error: None,
        },
    );
    store.save_state(&run).unwrap();
    (temp, store, run)
}

#[test]
fn web_stage_restart_uses_the_restart_transition() {
    let (_temp, store, run) = restart_fixture();
    let before = store.load_state(&run.id).unwrap().generation;
    super::apply_control(&store, restart_request(&run.id, "restart-stage", "call-a", None)).unwrap();
    assert_eq!(store.load_state(&run.id).unwrap().generation, before + 1);
}

#[test]
fn web_item_restart_uses_the_restart_transition() {
    let (_temp, store, run) = restart_fixture();
    let before = store.load_state(&run.id).unwrap().generation;
    super::apply_control(&store, restart_request(&run.id, "restart-item", "call-a", Some("item-a"))).unwrap();
    assert_eq!(store.load_state(&run.id).unwrap().generation, before + 1);
}

#[test]
fn web_restart_refuses_a_held_executor_lease_without_mutating_state() {
    let (_temp, store, run) = restart_fixture();
    let before = store.load_state(&run.id).unwrap();
    let lock = store.run_dir(&run.id).join("decomposition/executor.lock");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let holder = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock).unwrap();
    holder.try_lock().unwrap();
    let result = super::apply_control(&store, restart_request(&run.id, "restart-stage", "call-a", None));
    assert!(result.is_err());
    let after = store.load_state(&run.id).unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.stages, before.stages);
}
