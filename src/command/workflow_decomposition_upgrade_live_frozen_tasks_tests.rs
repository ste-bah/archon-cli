//! O3 on a run launched before the frozen entries were passed: its resume
//! record for this generation carries no `tasks`, and the resume reads them
//! from the published skeleton, so the resumed script re-authors only the
//! bodies a skeleton re-author reaches. The record itself is never rewritten.

use super::*;
use crate::command::workflow_decompose::repair_tests::set_gate_skeleton as gate;

#[tokio::test]
async fn a_resume_record_without_tasks_takes_them_from_the_published_skeleton() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let task_root = Path::new(&state.identity.task_root_identity);
    let prd = Path::new(&state.identity.prd_identity);
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_chain(
        project.path(),
        prd,
        task_root,
        &["TASK-X-010", "TASK-X-020"],
    );
    // Both freezes were judged by an older logic version: the resume
    // verifies the frozen chain in place instead of replaying them.
    let current = crate::command::workflow_host_command_logic::versions();
    for (capability, id) in [
        ("freeze-acceptance", "freeze-acceptance-1"),
        ("freeze-skeleton", "freeze-skeleton-1"),
    ] {
        super::record_freeze(
            &store,
            &run_id,
            project.path(),
            task_root,
            capability,
            id,
            "2026-10-10T10:00:00Z",
            "accepted",
            current[capability] - 1,
            serde_json::json!([]),
            true,
        );
    }
    // The record an older build wrote for this generation: no `tasks`.
    let generation = store.load_state(&run_id).unwrap().generation;
    let mut snapshot = serde_json::to_value(
        crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
            project.path(),
            prd,
            task_root,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(snapshot.as_object_mut().unwrap().remove("tasks").is_some());
    let relative = format!("decomposition/resume-frozen-chain/generation-{generation}.json");
    store
        .write_run_json(
            &run_id,
            &relative,
            &serde_json::json!({"schema_version": 1, "generation": generation, "snapshot": snapshot}),
        )
        .unwrap();
    let record = std::fs::read(store.run_dir(&run_id).join(&relative)).unwrap();

    let effective = crate::command::workflow_decompose_frozen_chain::resume_chain_argument(
        &store,
        &run_id,
        generation,
        project.path(),
        prd,
        task_root,
        false,
    )
    .unwrap();
    assert_eq!(effective["skeleton"], true, "{effective}");
    let published: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(
        &std::fs::read(task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        effective["tasks"],
        serde_json::json!(published.tasks),
        "{effective}"
    );
    assert_eq!(
        std::fs::read(store.run_dir(&run_id).join(&relative)).unwrap(),
        record,
        "the resume record is never rewritten"
    );

    // The resumed script: no body is on disk, so each is authored once; the
    // skeleton re-author changes only TASK-X-010's entry and adds none, so
    // only TASK-X-010 is authored again.
    let reauthored = gate::skeleton(vec![
        serde_json::json!({"task_id": "TASK-X-010", "file_name": "TASK-X-010.md",
            "implements": ["AC-X-001", "REQ-X-009"]}),
        serde_json::json!({"task_id": "TASK-X-020", "file_name": "TASK-X-020.md",
            "implements": ["AC-X-001"]}),
    ]);
    let out = gate::run(&gate::driver_on(
        &[reauthored],
        &[],
        "",
        gate::UNCLAIMED,
        &effective.to_string(),
    ));
    assert!(out.get("error").is_none(), "{out}");
    let authors = |task: &str| {
        let prefix = format!("body-{task}-author-");
        (gate::calls(&out, "agentCalls").iter())
            .filter(|id| id.starts_with(&prefix))
            .count()
    };
    assert_eq!(authors("TASK-X-010"), 2, "{out}");
    assert_eq!(authors("TASK-X-020"), 1, "{out}");
}
