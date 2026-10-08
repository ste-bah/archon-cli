use super::*;
use archon_workflow::bundle::{WorkflowBundle, WorkflowBundleOrigin};
use archon_workflow::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore,
};

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
    WorkflowBundle::create_for_run(
        &store,
        &run,
        "export default 1",
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
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

fn seed_call_cache(store: &archon_workflow::WorkflowStore, run: &archon_workflow::WorkflowRun) {
    let call = WorkflowV2HostCall {
        id: "call-a".into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    };
    let record = WorkflowV2CallRecord::new(
        run.id.clone(),
        call,
        1,
        "input-hash".into(),
        WorkflowV2Result::accepted("cached"),
        vec![],
    );
    WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .save_call_record(&record)
        .unwrap();
}

#[test]
fn web_stage_restart_uses_the_restart_transition() {
    let (_temp, store, run) = restart_fixture();
    seed_call_cache(&store, &run);
    let before = store.load_state(&run.id).unwrap().generation;
    super::apply_control(&store, restart_request(&run.id, "restart-stage", "call-a", None)).unwrap();
    assert_eq!(store.load_state(&run.id).unwrap().generation, before + 1);
    assert!(WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .load_call_record("call-a")
        .unwrap()
        .unwrap()
        .invalidated_by
        .is_some());
}

#[test]
fn web_item_restart_uses_the_restart_transition() {
    let (_temp, store, run) = restart_fixture();
    seed_call_cache(&store, &run);
    let before = store.load_state(&run.id).unwrap().generation;
    super::apply_control(&store, restart_request(&run.id, "restart-item", "call-a", Some("item-a"))).unwrap();
    assert_eq!(store.load_state(&run.id).unwrap().generation, before + 1);
    assert!(WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"))
        .load_call_record("call-a")
        .unwrap()
        .unwrap()
        .invalidated_by
        .is_some());
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
    assert!(matches!(
        result,
        Err(archon_workflow::WorkflowError::ControlCancelled(message))
            if message.contains("executor lease is held")
    ));
    let after = store.load_state(&run.id).unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.stages, before.stages);
}
