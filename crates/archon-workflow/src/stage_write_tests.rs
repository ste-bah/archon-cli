use super::*;
use crate::{LifecycleAction, LifecycleController, RunStatus, WorkflowSpec};

fn running(store: &WorkflowStore) -> (String, u64) {
    let mut run = store
        .create_run(WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.into(),
            name: "stage write".into(),
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
    (run.id, run.generation)
}

async fn obsolete_write(relative: &str, resume: bool) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    let path = store.run_dir(&run_id).join(relative);
    let writer = StageWriter {
        store: store.clone(),
        run_id: run_id.clone(),
        owner: PauseOwner::Generation(generation),
    };
    scope(writer, async {
        LifecycleController::new(store.clone())
            .apply(&run_id, LifecycleAction::Pause)
            .unwrap();
        if resume {
            LifecycleController::new(store.clone())
                .apply(&run_id, LifecycleAction::Resume)
                .unwrap();
        }
        let result = with_write(|| {
            crate::store::write_atomic(&path.with_extension("tmp"), &path, b"obsolete")
        });
        assert!(result.is_err(), "obsolete stage wrote {}", path.display());
        assert!(!path.exists());
    })
    .await;
}
#[tokio::test]
async fn gc_obsolete_stage_cannot_write_evidence() {
    obsolete_write("evidence.stdout", true).await;
}
#[tokio::test]
async fn gc_obsolete_stage_cannot_write_grants() {
    obsolete_write("scope-grants.json", true).await;
}
#[tokio::test]
async fn gc_paused_stage_cannot_publish_contract() {
    obsolete_write("acceptance-contract.json", false).await;
}

#[tokio::test]
async fn gc_takeover_cannot_interleave_a_stage_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    let writer = StageWriter {
        store: store.clone(),
        run_id: run_id.clone(),
        owner: PauseOwner::Generation(generation),
    };
    let path = store.run_dir(&run_id).join("atomic-mutation");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    scope(writer, async {
        let mut takeover = None;
        with_write(|| {
            let store = store.clone();
            let run_id = run_id.clone();
            takeover = Some(std::thread::spawn(move || {
                started_tx.send(()).unwrap();
                let control = LifecycleController::new(store);
                control.apply(&run_id, LifecycleAction::Pause).unwrap();
                control.apply(&run_id, LifecycleAction::Resume).unwrap();
                let _ = done_tx.send(());
            }));
            started_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            assert!(
                done_rx
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .is_err(),
                "takeover interleaved the mutation"
            );
            crate::store::write_atomic(&path.with_extension("tmp"), &path, b"owned")
        })
        .unwrap();
        takeover.unwrap().join().unwrap();
        let stale = with_write(|| {
            crate::store::write_atomic(&path.with_extension("tmp"), &path, b"obsolete")
        });
        assert!(stale.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"owned");
    })
    .await;
}

async fn tripwire_case(case: u8) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    let root = store.run_dir(&run_id);
    let input = temp.path().join("inputs/config.json");
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    std::fs::write(&input, b"before").unwrap();
    crate::write_coordinator::project_inputs::write_test_policy(&root, temp.path(), &["inputs"]);
    let writer = StageWriter {
        store: store.clone(),
        run_id: run_id.clone(),
        owner: PauseOwner::Generation(generation),
    };
    let control = LifecycleController::new(store);
    if case == 0 {
        control.apply(&run_id, LifecycleAction::Pause).unwrap();
        control.apply(&run_id, LifecycleAction::Resume).unwrap();
    }
    let result = crate::write_coordinator::input_tripwire::watch_owned(
        writer,
        Some(&root),
        "owned tripwire",
        async {
            if case != 0 {
                control.apply(&run_id, LifecycleAction::Pause).unwrap();
                if case == 2 {
                    control.apply(&run_id, LifecycleAction::Resume).unwrap();
                }
                std::fs::write(&input, b"after").unwrap();
            }
        },
    )
    .await;
    assert!(
        result.is_err(),
        "obsolete tripwire mutated shared evidence or inputs"
    );
    if case == 0 {
        assert!(
            !root
                .join("write-coordination/input-tripwire/objects")
                .exists()
        );
    } else {
        assert_eq!(std::fs::read(input).unwrap(), b"after");
        assert!(
            !root
                .join("write-coordination/environment-violations.jsonl")
                .exists()
        );
        assert!(
            !root
                .join("write-coordination/environment-violations")
                .exists()
        );
    }
}
#[tokio::test]
async fn gc_tripwire_stale_arm_writes_nothing() {
    tripwire_case(0).await;
}
#[tokio::test]
async fn gc_tripwire_paused_check_writes_nothing() {
    tripwire_case(1).await;
}
#[tokio::test]
async fn gc_tripwire_resumed_check_writes_nothing() {
    tripwire_case(2).await;
}
