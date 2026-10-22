use super::*;

#[test]
fn the_whole_set_publishers_take_the_chain_lock() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", true)]);
    let held = ChainLock::acquire(&set.pin_path(), &set.tasks).unwrap();
    let pin = set.pin();
    let lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(set.tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap())
            .unwrap();
    let contract_bytes = set.contract_bytes();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
    std::thread::scope(|scope| {
        let publisher = scope.spawn(|| {
            started_tx.send(()).unwrap();
            let result = publish::publish_acceptance_files(
                &set.tasks,
                set.project.path(),
                &contract_bytes,
                &lock,
                &pin,
            )
            .map_err(|error| error.to_string());
            result_tx.send(result).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(matches!(
            result_rx.recv_timeout(std::time::Duration::from_millis(150)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(held);
        result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("acceptance publisher resumes after the chain lock is released")
            .expect("acceptance publication succeeds");
        publisher.join().unwrap();
    });

    let _held = ChainLock::acquire(&set.pin_path(), &set.tasks).unwrap();
    let skeleton_lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(set.tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap())
            .unwrap();
    let error = publish::publish_skeleton_files(
        &set.tasks,
        &set.pin_path(),
        &std::fs::read(set.tasks.join(TASK_SKELETON_FILE)).unwrap(),
        &skeleton_lock,
        &pin,
    )
    .expect_err("skeleton publisher waits for the lock")
    .to_string();
    assert!(error.contains("another freeze or repair"), "{error}");
}

#[test]
fn a_target_that_moved_since_verification_is_never_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("chain.json");
    std::fs::write(&target, b"someone else's").unwrap();
    let anchor = tempfile::tempdir().unwrap();
    let error = begin_publish(
        &anchor.path().join("pin.json"),
        temp.path(),
        &[(target.clone(), b"ours".to_vec())],
        "test",
        &[(target.clone(), Some(content_digest(b"what we verified")))],
    )
    .err()
    .expect("moved target")
    .to_string();
    assert!(error.contains("changed since it was verified"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"someone else's");
    assert_eq!(
        std::fs::read_dir(temp.path()).unwrap().count(),
        1,
        "no temps left"
    );
}

#[test]
fn a_stale_whole_set_source_is_rejected_without_replacing_the_current_contract() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp
        .path()
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let current = b"current per-check repair";
    std::fs::write(&target, current).unwrap();
    let anchor = tempfile::tempdir().unwrap();
    let error = begin_publish(
        &anchor.path().join("pin.json"),
        temp.path(),
        &[(target.clone(), b"prepared stale contract".to_vec())],
        "workflow freeze-acceptance",
        &[(target.clone(), Some(content_digest(b"prepared source")))],
    )
    .err()
    .expect("stale whole-set source is detected at commit");
    assert!(
        error
            .downcast_ref::<crate::command::workflow_task_set::StaleAcceptanceContract>()
            .is_some()
    );
    assert_eq!(std::fs::read(&target).unwrap(), current);
}

#[tokio::test]
async fn a_whole_set_freeze_keeps_bytes_written_by_a_real_per_check_republish() {
    use archon_workflow::task_set_contract::AcceptanceCheck;

    let mut set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", true)]);
    std::fs::write(set.project.path().join("out.json"), r#"{"a":true,"b":1}"#).unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(
        set.project.path(),
    );
    let baseline =
        crate::command::workflow_task_set::executability::Baseline::head_of(set.project.path())
            .unwrap();
    set.probe = set.probe.with_baseline(baseline);

    let ids = ids(&["AC-F-001"]);
    let repaired_command = "jq -e '.a == true and .b == 1' out.json";
    let author = accept_all(repaired_command);
    reauthor_and_republish(&author, request(&set, &ids), &scope(&set))
        .await
        .expect("the per-check republish publishes its judged and proven repair");
    let repaired_bytes = set.contract_bytes();
    let repaired: AcceptanceContract = serde_json::from_slice(&repaired_bytes).unwrap();
    let AcceptanceCheck::Command { command, .. } = &repaired.acceptance[0].check else {
        panic!("the republished entry remains a command check");
    };
    assert_eq!(command, repaired_command);

    let prepared = crate::command::workflow_task_set::prepare_acceptance_freeze(
        set.project.path(),
        &set.tasks,
        &set.prd,
        archon_core::config::GateMode::Observe,
        std::sync::Arc::new(ScriptedAuthorJudge::new(
            |entry, _| serde_json::to_string(entry).unwrap(),
            |_, _| true,
        )),
    )
    .await
    .unwrap();
    let prepared_contract = prepared.contract().unwrap();
    let AcceptanceCheck::Command { command, .. } = &prepared_contract.acceptance[0].check else {
        panic!("the whole-set candidate retains the repaired entry");
    };
    assert_eq!(command, repaired_command);
    assert_eq!(prepared.candidate_digest, content_digest(&repaired_bytes));
    assert_eq!(
        prepared.live_contract_digest,
        Some(content_digest(&repaired_bytes))
    );
}

#[test]
fn rollback_never_restores_over_a_target_another_writer_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("chain.json");
    std::fs::write(&target, b"old").unwrap();
    let anchor = tempfile::tempdir().unwrap();
    let transaction = begin_publish(
        &anchor.path().join("pin.json"),
        temp.path(),
        &[(target.clone(), b"ours".to_vec())],
        "test",
        &[],
    )
    .unwrap();
    std::fs::write(&target, b"theirs").unwrap();
    let error = transaction
        .roll_back()
        .expect_err("not ours to undo")
        .to_string();
    assert!(error.contains("changed after this publish"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"theirs");
}
