//! `observe-run-end` on a finished run whose observer failed because its
//! pin moved with no recorded lineage: refused before any write while the
//! chain is unproven, then re-run observe-only once the launch versions are
//! imported, changing nothing but the observer's own records.

use super::super::*;
use crate::command::acceptance_chain::import_history;
use crate::command::workflow_live::reobserve::observe_run_end;
use archon_workflow::task_skeleton::TaskSkeleton;

fn read(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

fn write_json<T: serde::Serialize>(path: &std::path::Path, value: &T) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

/// Re-author `id`'s check in place and re-lock the whole chain, as an
/// older binary's per-check repair did: no lineage, nothing filed.
fn move_pin_without_lineage(fixture: &FrozenFixture, id: &str, check: AcceptanceCheck) {
    let tasks = &fixture.task_root;
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&read(&tasks.join(ACCEPTANCE_CONTRACT_FILE))).unwrap();
    for entry in contract
        .acceptance
        .iter_mut()
        .filter(|entry| entry.id == id)
    {
        entry.check = check.clone();
        entry.judgment.reason = "re-judged after the re-author".into();
    }
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let digest = content_digest(&bytes);
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
    let mut lock: AcceptanceLock =
        serde_json::from_slice(&read(&tasks.join(ACCEPTANCE_LOCK_FILE))).unwrap();
    lock.digest.clone_from(&digest);
    write_json(&tasks.join(ACCEPTANCE_LOCK_FILE), &lock);
    let skeleton_path = tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    let mut skeleton: TaskSkeleton = serde_json::from_slice(&read(&skeleton_path)).unwrap();
    skeleton.acceptance_digest.clone_from(&digest);
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let skeleton_digest = content_digest(&skeleton_bytes);
    std::fs::write(&skeleton_path, &skeleton_bytes).unwrap();
    let skeleton_lock_path =
        tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE);
    let mut skeleton_lock: archon_workflow::task_skeleton::TaskSkeletonLock =
        serde_json::from_slice(&read(&skeleton_lock_path)).unwrap();
    skeleton_lock.digest.clone_from(&skeleton_digest);
    skeleton_lock.acceptance_digest.clone_from(&digest);
    write_json(&skeleton_lock_path, &skeleton_lock);
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(fixture.project.path(), tasks);
    let mut pin: AcceptancePin = serde_json::from_slice(&read(&pin_path)).unwrap();
    pin.freeze_event_id = format!("acceptance-freeze-{}", &digest[..12]);
    pin.acceptance_digest = digest;
    pin.skeleton_digest = Some(skeleton_digest);
    write_json(&pin_path, &pin);
}

fn finalization(fixture: &FrozenFixture, run_id: &str) -> archon_workflow::FinalizationRecordV1 {
    serde_json::from_slice(&read(
        &fixture.store.run_dir(run_id).join(FINALIZATION_RECORD_PATH),
    ))
    .unwrap()
}

#[tokio::test]
async fn observe_run_end_refuses_an_unproven_chain_then_reobserves_once_the_launch_is_imported() {
    let fixture = frozen_fixture(vec![criterion("AC-X-001", floor("missing.json"))]);
    let tasks = fixture.task_root.clone();
    let launch_contract = read(&tasks.join(ACCEPTANCE_CONTRACT_FILE));
    let launch_skeleton = read(&tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE));
    let run = fixture.store.create_run(finalizer_spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(fixture.store.run_dir(&run.id).join("v2"));
    seed_finalizer_call(&v2_store);
    move_pin_without_lineage(&fixture, "AC-X-001", floor("missing-too.json"));
    let observer = FixedRunEndAcceptanceObserver::new(fixture.store.clone());
    finalize_summary(
        &fixture.store,
        &run.id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(fixture.snapshot.clone()),
        &finalizer_summary(),
        &v2_store,
        Some(&observer),
        None,
    )
    .await
    .unwrap();
    let Some(RunEndObserverStateV1::Failed { reason }) =
        finalization(&fixture, &run.id).observer_state
    else {
        panic!("the moved pin fails the run-end observation");
    };
    assert!(
        reason.contains("chain check unrecorded_change failed"),
        "{reason}"
    );
    let state_path = fixture.store.run_dir(&run.id).join("state.json");
    let state_before = read(&state_path);
    let record_path = fixture
        .store
        .run_dir(&run.id)
        .join(FINALIZATION_RECORD_PATH);
    let record_before = read(&record_path);
    let chain_before: Vec<Vec<u8>> = std::fs::read_dir(&tasks)
        .unwrap()
        .map(|entry| read(&entry.unwrap().path()))
        .collect();

    let refused = observe_run_end(fixture.project.path(), &run.id)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("unrecorded_change"), "{refused}");
    assert!(
        refused.contains(&format!(
            "archon workflow import-chain-history {} --from",
            run.id
        )),
        "{refused}"
    );
    assert_eq!(
        read(&record_path),
        record_before,
        "an unproven chain writes nothing"
    );

    let saved = tempfile::tempdir().unwrap();
    let (contract_file, skeleton_file) = (saved.path().join("c.json"), saved.path().join("s.json"));
    std::fs::write(&contract_file, &launch_contract).unwrap();
    std::fs::write(&skeleton_file, &launch_skeleton).unwrap();
    let imported = import_history(
        fixture.project.path(),
        &run.id,
        &[contract_file, skeleton_file],
    )
    .unwrap();
    assert!(imported.iter().all(|file| file.stored));

    let report = observe_run_end(fixture.project.path(), &run.id)
        .await
        .unwrap();
    assert!(
        report.starts_with("observer: completed authority=observe_only"),
        "{report}"
    );
    let record = finalization(&fixture, &run.id);
    assert!(matches!(
        record.observer_state,
        Some(RunEndObserverStateV1::Completed { .. })
    ));
    assert_eq!(record.prior_observer_failures, vec![reason]);
    assert_eq!(record.terminal_status, RunStatus::Completed);
    assert_eq!(
        read(&state_path),
        state_before,
        "the run's state is never written"
    );
    let chain_after: Vec<Vec<u8>> = std::fs::read_dir(&tasks)
        .unwrap()
        .map(|entry| read(&entry.unwrap().path()))
        .collect();
    assert_eq!(
        chain_after, chain_before,
        "nothing under the task root changes"
    );
    let labels: Vec<String> = read_events(&fixture.store, &run.id)
        .iter()
        .map(|event| {
            event.detail["event"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert!(
        labels
            .iter()
            .any(|label| label == "acceptance_chain_history_imported")
    );
    assert!(
        labels
            .iter()
            .any(|label| label == "run_end_acceptance_observer_reopened")
    );

    let again = observe_run_end(fixture.project.path(), &run.id)
        .await
        .unwrap_err()
        .to_string();
    assert!(again.contains("reopened only after it failed"), "{again}");
}
