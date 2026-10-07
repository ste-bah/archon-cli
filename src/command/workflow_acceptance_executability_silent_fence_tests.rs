use super::*;
use archon_workflow::control_pause::PauseOwner;
use archon_workflow::stage_write::{StageWriter, scope};
use archon_workflow::{
    LifecycleAction, LifecycleController, RunStatus, WorkflowSpec, WorkflowStore,
};

async fn takeover(case: u8) {
    let project = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(project.path());
    let mut run = store
        .create_run(WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "probe".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();
    let writer = StageWriter {
        store: store.clone(),
        run_id: run.id.clone(),
        owner: PauseOwner::Generation(run.generation),
    };
    let probe = HostProbe::at(project.path().into(), project.path().into(), None);
    let command = if case == 2 {
        "test -f feature"
    } else {
        "r3-unavailable-tool"
    };
    let contract = crate::command::workflow_task_set::republish::test_fixture::frozen_set(&[(
        "check", command, true,
    )])
    .contract();
    let path = strike(&probe, "abcdef", &contract, "check");
    if case != 0 {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"current owner's first strike").unwrap();
    }
    scope(writer, async {
        // The production boundary: ownership changes while verdict classification is awaited.
        let at = context(&probe, &contract);
        let result = CheckResult {
            acceptance_id: "check".into(),
            exit_code: Some(if case == 2 { 1 } else { 127 }),
            quota_walk_count: 0,
            stdout: vec![],
            stderr: if case == 2 {
                vec![]
            } else {
                b"sh: r3-unavailable-tool: command not found\n".to_vec()
            },
            operational_error: None,
            classification: None,
        };
        let control = LifecycleController::new(store.clone());
        let (classified, _) =
            tokio::join!(silent_failure_off_thread(&contract, &result, &at), async {
                control.apply(&run.id, LifecycleAction::Pause).unwrap();
                control.apply(&run.id, LifecycleAction::Resume).unwrap();
            });
        if case == 2 {
            assert!(classified.is_none(), "the assertion failure is a verdict");
            clear(&probe, "abcdef", &contract, "check");
        } else {
            let finding = settle(
                &probe,
                "abcdef",
                &contract,
                &result,
                classified
                    .as_deref()
                    .expect("missing tool must give no verdict"),
            );
            assert!(
                finding.is_none(),
                "obsolete reader classified a first strike as repeated"
            );
        }
    })
    .await;
    if case == 0 {
        assert!(
            !path.exists(),
            "obsolete classification saved a shared strike"
        );
    } else {
        assert_eq!(
            std::fs::read(path).unwrap(),
            b"current owner's first strike"
        );
    }
}
#[tokio::test]
async fn r3_takeover_during_classification_cannot_save_strike() {
    takeover(0).await;
}
#[tokio::test]
async fn r3_takeover_during_classification_cannot_reuse_strike() {
    takeover(1).await;
}
#[tokio::test]
async fn r3_takeover_during_classification_cannot_delete_strike() {
    takeover(2).await;
}
