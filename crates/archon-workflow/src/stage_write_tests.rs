use super::*;
use crate::{LifecycleAction, LifecycleController, RunStatus, WorkflowSpec};

#[test]
fn file_removal_syncs_its_parent_directory() {
    for relative in ["strike", "nested/strike", "deeper/nested/strike"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"first strike").unwrap();
        crate::durable_io::take_synced();

        crate::stage_write::remove_file(&path).unwrap();

        assert!(!path.exists(), "{} remains", path.display());
        assert_eq!(
            crate::durable_io::take_synced(),
            vec![path.parent().unwrap().to_path_buf()],
            "{} was removed without durably syncing its parent",
            path.display()
        );
    }
}

#[test]
fn removing_an_absent_file_does_not_claim_a_durable_unlink() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("absent-strike");
    crate::durable_io::take_synced();

    crate::stage_write::remove_file(&path).unwrap();

    assert!(crate::durable_io::take_synced().is_empty());
}

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

#[path = "stage_write_cache_tests.rs"]
mod cache_tests;

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
                let control = LifecycleController::new(store.clone());
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
    let control = LifecycleController::new(store.clone());
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
        if case == 1 {
            control.apply(&run_id, LifecycleAction::Resume).unwrap();
        }
        let current = StageWriter {
            store: store.clone(),
            run_id: run_id.clone(),
            owner: PauseOwner::Generation(store.load_state(&run_id).unwrap().generation),
        };
        let called = std::sync::atomic::AtomicBool::new(false);
        let reconciled = scope(current.clone(), async {
            // Round entry must reconcile before reserving evidence or running
            // any of the stage's pre-evaluation repairs.
            crate::v2::acceptance_stage::reserve_round(&root, 1)?;
            crate::write_coordinator::input_tripwire::watch_owned(
                current,
                Some(&root),
                "next evaluation",
                async {
                    called.store(true, std::sync::atomic::Ordering::SeqCst);
                },
            )
            .await
        })
        .await;
        assert!(
            matches!(reconciled, Err(WorkflowError::ControlPaused(_))),
            "pending comparison was lost"
        );
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(
            !root
                .join(crate::v2::acceptance_stage::ACCEPTANCE_RECORDS_DIR)
                .exists(),
            "round reserved evidence before reconciling"
        );
        assert_eq!(std::fs::read(input).unwrap(), b"before");
        assert!(
            root.join("write-coordination/environment-violations.jsonl")
                .exists()
        );
        assert!(
            root.join("write-coordination/environment-violations")
                .exists()
        );
    }
}
#[tokio::test]
async fn gc_tripwire_stale_arm_writes_nothing() {
    tripwire_case(0).await;
}
#[tokio::test]
async fn r2_tripwire_paused_comparison_reconciles_before_new_work() {
    tripwire_case(1).await;
}
#[tokio::test]
async fn r2_tripwire_superseded_comparison_reconciles_before_new_work() {
    tripwire_case(2).await;
}

#[tokio::test]
async fn r2_tripwire_dropped_call_reconciles_before_new_work() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    let root = store.run_dir(&run_id);
    let input = temp.path().join("inputs/config.json");
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    std::fs::write(&input, b"before").unwrap();
    crate::write_coordinator::project_inputs::write_test_policy(&root, temp.path(), &["inputs"]);
    let writer = StageWriter {
        store,
        run_id,
        owner: PauseOwner::Generation(generation),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut first = Box::pin(crate::write_coordinator::input_tripwire::watch_owned(
        writer.clone(),
        Some(&root),
        "dropped evaluation",
        async {
            std::fs::remove_file(&input).unwrap();
            tx.send(()).unwrap();
            std::future::pending::<()>().await;
        },
    ));
    tokio::select! { _ = &mut first => panic!("call completed"), _ = rx => {} }
    // Drop the entire armed future, as executor cancellation does.
    drop(first);
    let called = std::sync::atomic::AtomicBool::new(false);
    let result = crate::write_coordinator::input_tripwire::watch_owned(
        writer,
        Some(&root),
        "next evaluation",
        async {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
        },
    )
    .await;
    assert!(
        matches!(result, Err(WorkflowError::ControlPaused(_))),
        "cancelled call lost its comparison"
    );
    assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(std::fs::read(input).unwrap(), b"before");
}

#[tokio::test]
async fn r2_tripwire_process_restart_keeps_comparison_and_requires_repair() {
    use crate::write_coordinator::input_tripwire::watch_owned;
    if let Ok(project) = std::env::var("ARCHON_R2_TRIPWIRE_CHILD_PROJECT") {
        let store = WorkflowStore::project(&project);
        let run_id = std::env::var("ARCHON_R2_TRIPWIRE_CHILD_RUN").unwrap();
        let generation = store.load_state(&run_id).unwrap().generation;
        let root = store.run_dir(&run_id);
        let writer = StageWriter {
            store,
            run_id,
            owner: PauseOwner::Generation(generation),
        };
        let input = std::path::Path::new(&project).join("inputs/config.json");
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut call = Box::pin(watch_owned(
            writer,
            Some(&root),
            "previous process",
            async {
                std::fs::write(input, b"changed in child").unwrap();
                tx.send(()).unwrap();
                std::future::pending::<()>().await;
            },
        ));
        tokio::select! { _ = &mut call => panic!("call completed"), _ = rx => {} }
        drop(call);
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let (run_id, generation) = running(&store);
    let root = store.run_dir(&run_id);
    let input = temp.path().join("inputs/config.json");
    std::fs::create_dir_all(input.parent().unwrap()).unwrap();
    std::fs::write(&input, b"before").unwrap();
    crate::write_coordinator::project_inputs::write_test_policy(&root, temp.path(), &["inputs"]);
    let name = std::thread::current().name().unwrap().to_string();
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args(["--exact", &name])
        .env("ARCHON_R2_TRIPWIRE_CHILD_PROJECT", temp.path())
        .env("ARCHON_R2_TRIPWIRE_CHILD_RUN", &run_id)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "child evaluation fixture failed");
    assert_eq!(
        std::fs::read(&input).unwrap(),
        b"changed in child",
        "the child reached its watched mutation"
    );
    let writer = StageWriter {
        store,
        run_id,
        owner: PauseOwner::Generation(generation),
    };
    let called = std::sync::atomic::AtomicBool::new(false);
    let result = watch_owned(writer.clone(), Some(&root), "next process", async {
        called.store(true, std::sync::atomic::Ordering::SeqCst);
    })
    .await;
    let Err(WorkflowError::ControlPaused(reason)) = result else {
        panic!("a process restart lost the original comparison");
    };
    assert!(reason.contains("previous process"));
    assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    // Host-write history is process-local: an uncertain repair keeps the
    // original map and pauses again instead of adopting the changed bytes.
    assert_eq!(std::fs::read(&input).unwrap(), b"changed in child");
    assert!(
        watch_owned(writer.clone(), Some(&root), "retry", async {})
            .await
            .is_err()
    );
    std::fs::write(&input, b"before").unwrap();
    assert!(
        watch_owned(
            writer.clone(),
            Some(&root),
            "repair acknowledgement",
            async {}
        )
        .await
        .is_err(),
        "the earlier detection still needs reporting after repair"
    );
    assert!(
        watch_owned(writer, Some(&root), "after reconciliation", async {})
            .await
            .is_ok()
    );
}
