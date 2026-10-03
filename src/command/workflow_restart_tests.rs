//! Issue-256/267: a restart is refused while a live executor holds the run's
//! executor lease, and proceeds once the lease is free; and a restart-stage
//! never commits a rewound state over a valid V2 cache.

use super::*;
use crate::command::workflow_executor_lease;

fn run_with_stage(temp: &tempfile::TempDir) -> (WorkflowStore, WorkflowRun) {
    run_with_stage_named(temp, "build")
}

fn is_live_refusal(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("is live") && text.contains("restart")
}

#[test]
fn restart_task_is_refused_while_a_live_executor_holds_the_lease() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = run_with_stage(&temp);
    let lease = workflow_executor_lease::acquire(&store.run_dir(&run.id), &run.id).unwrap();
    let before = store.load_state(&run.id).unwrap();

    let refused = restart_task_workflow(&store, &run.id, "build").unwrap_err();
    assert!(is_live_refusal(&refused), "{refused:#}");
    assert_eq!(
        store.load_state(&run.id).unwrap().generation,
        before.generation,
        "a refused restart writes nothing"
    );

    drop(lease);
    restart_task_workflow(&store, &run.id, "build").expect("lease free: restart proceeds");
    assert!(store.load_state(&run.id).unwrap().generation > before.generation);
}

#[test]
fn restart_stage_and_agent_are_refused_while_a_live_executor_holds_the_lease() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = run_with_stage(&temp);
    let lease = workflow_executor_lease::acquire(&store.run_dir(&run.id), &run.id).unwrap();
    let before = store.load_state(&run.id).unwrap().generation;

    for action in [
        LifecycleAction::RestartStage("build".to_string()),
        LifecycleAction::RestartItem {
            stage_id: "build".to_string(),
            item_id: "item-1".to_string(),
        },
    ] {
        let refused = lifecycle(&store, &run.id, action).unwrap_err();
        assert!(is_live_refusal(&refused), "{refused:#}");
    }
    assert_eq!(store.load_state(&run.id).unwrap().generation, before);

    drop(lease);
    lifecycle(
        &store,
        &run.id,
        LifecycleAction::RestartStage("build".to_string()),
    )
    .expect("lease free: restart proceeds");
    assert!(store.load_state(&run.id).unwrap().generation > before);
}

/// A generated V2 run whose stage `author-a` has an accepted cached record.
fn generated_run_with_cached_stage(
    temp: &tempfile::TempDir,
) -> (
    WorkflowStore,
    WorkflowRun,
    archon_workflow::WorkflowV2ResultStore,
) {
    let (store, run) = run_with_stage_named(temp, "author-a");
    archon_workflow::WorkflowBundle::create_for_run(
        &store,
        &run,
        "export default async function workflow(w) {}",
        WorkflowBundleOrigin::GeneratedHarness,
    )
    .unwrap();
    let call = archon_workflow::WorkflowV2HostCall {
        id: "author-a".to_string(),
        method: archon_workflow::WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    };
    let v2_root = store.run_dir(&run.id).join("v2");
    std::fs::create_dir_all(&v2_root).unwrap();
    std::fs::write(
        v2_root.join("generated-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "generated_scaffold": { "host_call_manifest": [call.clone()] },
        }))
        .unwrap(),
    )
    .unwrap();
    let v2 = archon_workflow::WorkflowV2ResultStore::new(v2_root);
    let record = archon_workflow::WorkflowV2CallRecord::new(
        &run.id,
        call,
        1,
        "in-author-a".to_string(),
        archon_workflow::WorkflowV2Result::accepted("done"),
        vec![],
    );
    v2.save_call_record(&record).unwrap();
    (store, run, v2)
}

fn run_with_stage_named(temp: &tempfile::TempDir, stage: &str) -> (WorkflowStore, WorkflowRun) {
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml(&format!(
        "schema: archon.workflow.v1\nname: restart-atomic\ntask: test\nstages:\n  - id: {stage}\n    kind: agent\n"
    ))
    .expect("spec");
    let run = store.create_run(spec).expect("run");
    (store, run)
}

/// Issue-267: the rewind and the cache invalidation commit as one unit. With
/// the event log broken, the restart reports an error after the state
/// commit; the cache it implies must already be invalidated, never a
/// rewound state over a valid cache.
#[test]
fn restart_stage_never_leaves_a_rewound_state_over_a_valid_cache() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run, v2) = generated_run_with_cached_stage(&temp);
    let before = store.load_state(&run.id).unwrap().generation;
    let events = store.run_dir(&run.id).join("events.jsonl");
    std::fs::remove_file(&events).unwrap();
    std::fs::create_dir(&events).unwrap();

    let result = lifecycle(
        &store,
        &run.id,
        LifecycleAction::RestartStage("author-a".to_string()),
    );

    let state = store.load_state(&run.id).unwrap();
    let record = v2.load_call_record("author-a").unwrap().expect("record");
    let rewound = state.generation != before;
    assert!(
        !rewound || record.invalidated_by.is_some(),
        "state rewound (generation {before} -> {}) but the cached record is still valid; result: {result:?}",
        state.generation
    );
    assert!(result.is_err(), "the broken event log is reported");
}

/// Round 2 of Issue-256: a run without a lease file is still restarted
/// under the lease, so a resume that starts in the middle of the restart is
/// refused instead of racing it.
#[test]
fn a_resume_cannot_start_inside_a_restart_of_a_run_without_a_lease_file() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run) = run_with_stage(&temp);
    let lease = store.run_dir(&run.id).join(workflow_executor_lease::LEASE);
    assert!(!lease.exists());

    with_restart_lease(&store, &run.id, || {
        let resumed = crate::command::workflow_task_root_reclaim::begin_execution(&store, &run.id);
        assert!(resumed.is_err(), "an executor started inside the restart");
        Ok(())
    })
    .unwrap();

    crate::command::workflow_task_root_reclaim::begin_execution(&store, &run.id)
        .expect("after the restart the lease is free");
}
