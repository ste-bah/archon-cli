use super::*;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptancePin, FreezeGateMode, FreezeGateStamp, content_digest,
    empty_gate_findings_digest,
};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn task_universe(task_root: &std::path::Path) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".into(),
        source_roots: vec![task_root.display().to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-EX-001".into(),
            source_path: task_root.join("TASK-EX-001.md").display().to_string(),
            ..WorkflowV2TaskUniverseTask::default()
        }],
    }
}

fn plan(universe: WorkflowV2TaskUniverse) -> WorkflowScriptPlan {
    WorkflowScriptPlan::generated(
        "implement decomposed tasks",
        "export default async function workflow(w) { await w.checkpoint(\"done\", {}); }",
        Vec::new(),
        Some(universe),
        GeneratedWorkflowConfig::default(),
        &archon_core::config::LearningConfig::default(),
    )
    .expect("plan resolves")
}

fn metadata_json(store: &WorkflowStore, run_id: &str) -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join(GENERATED_V2_METADATA_PATH)).expect("metadata"),
    )
    .expect("metadata json")
}

#[test]
fn absent_freeze_chain_keeps_observer_snapshot_omitted() {
    let project = tempfile::tempdir().expect("project");
    let task_root = project.path().join("tasks/set");
    std::fs::create_dir_all(&task_root).expect("tasks");
    std::fs::write(task_root.join("TASK-EX-001.md"), "task").expect("task");
    let store = WorkflowStore::project(project.path());
    let run = store
        .create_run(plan(task_universe(&task_root)).approval_metadata_spec())
        .unwrap();

    save_generated_v2_metadata(&store, &run.id, &plan(task_universe(&task_root)), true).unwrap();

    let json = metadata_json(&store, &run.id);
    assert!(json.get("observer_snapshot").is_none(), "{json:#}");
}

#[test]
fn any_freeze_chain_artifact_persists_expected_snapshot() {
    let project = tempfile::tempdir().expect("project");
    let task_root = project.path().join("tasks/set");
    std::fs::create_dir_all(&task_root).expect("tasks");
    std::fs::write(task_root.join("TASK-EX-001.md"), "task").expect("task");
    std::fs::write(task_root.join(ACCEPTANCE_CONTRACT_FILE), b"{}").expect("chain artifact");
    let store = WorkflowStore::project(project.path());
    let run = store
        .create_run(plan(task_universe(&task_root)).approval_metadata_spec())
        .unwrap();

    save_generated_v2_metadata(&store, &run.id, &plan(task_universe(&task_root)), true).unwrap();

    let json = metadata_json(&store, &run.id);
    let snapshot = &json["observer_snapshot"];
    assert_eq!(snapshot["schema_version"], 1);
    // A launch from now on records that it binds the run to recorded lineage.
    assert_eq!(
        snapshot["lineage_recording"],
        archon_workflow::task_set_lineage::LINEAGE_RECORDING_V1
    );
    assert_eq!(
        snapshot["canonical_task_root_identity"],
        task_root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(
        snapshot["expected_artifact_paths"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn portable_pin_identity_is_snapshotted_when_readable() {
    let project = tempfile::tempdir().expect("project");
    let task_root = project.path().join("tasks/set");
    std::fs::create_dir_all(&task_root).expect("tasks");
    std::fs::write(task_root.join("TASK-EX-001.md"), "task").expect("task");
    let digest = content_digest(b"acceptance");
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(project.path(), &task_root);
    std::fs::create_dir_all(pin_path.parent().unwrap()).expect("pin dir");
    std::fs::write(
        pin_path,
        serde_json::to_vec_pretty(&AcceptancePin {
            check_sources_digest: None,
            task_root: task_root
                .canonicalize()
                .map(archon_shell::paths::plain)
                .unwrap()
                .display()
                .to_string(),
            acceptance_digest: digest.clone(),
            freeze_event_id: "freeze-identity".into(),
            acceptance_gate: FreezeGateStamp {
                mode: FreezeGateMode::Observe,
                finding_count: 0,
                findings_digest: empty_gate_findings_digest(),
                binary_commit: "test".into(),
                evaluated_at: "2026-08-27T00:00:00Z".into(),
            },
            skeleton_digest: Some("skeleton".into()),
            skeleton_gate: None,
            fidelity_waivers: Vec::new(),
            lineage: Vec::new(),
            lineage_recording: None,
        })
        .unwrap(),
    )
    .unwrap();
    let store = WorkflowStore::project(project.path());
    let run = store
        .create_run(plan(task_universe(&task_root)).approval_metadata_spec())
        .unwrap();

    save_generated_v2_metadata(&store, &run.id, &plan(task_universe(&task_root)), true).unwrap();

    let json = metadata_json(&store, &run.id);
    assert_eq!(
        json["observer_snapshot"]["portable_acceptance_identity"]["freeze_event_id"],
        "freeze-identity"
    );
    assert_eq!(
        json["observer_snapshot"]["portable_acceptance_identity"]["acceptance_digest"],
        digest
    );
}

/// Issue 338: the launch snapshot binds the run to the pin it reads, so it
/// never reads a set left mid-publish. One a read can settle is settled
/// first and snapshotted whole; one no read can settle stops the launch with
/// the reason before any run is created; once fixed, the launch goes on.
#[test]
fn a_launch_snapshot_settles_a_left_publish_or_stops_with_the_reason() {
    crate::command::workflow_task_set::register_publish_settle();
    for stuck in [false, true] {
        let project = tempfile::tempdir().expect("project");
        let task_root = project.path().join("tasks/set");
        std::fs::create_dir_all(&task_root).expect("tasks");
        std::fs::write(task_root.join("TASK-EX-001.md"), "task").expect("task");
        let plain = task_root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap();
        let pin_path =
            crate::command::workflow_task_set::acceptance_pin_path(project.path(), &plain);
        std::fs::create_dir_all(pin_path.parent().unwrap()).expect("pin dir");
        let pin = |freeze_event_id: &str| {
            serde_json::to_vec_pretty(&serde_json::json!({
                "task_root": plain, "acceptance_digest": "d", "freeze_event_id": freeze_event_id,
                "acceptance_gate": {"mode": "observe", "finding_count": 0, "findings_digest": "f",
                    "binary_commit": "b", "evaluated_at": "t"}
            }))
            .unwrap()
        };
        std::fs::write(&pin_path, pin("old")).unwrap();
        let fix = crate::command::workflow_task_set::crash_publish(
            &pin_path,
            &[(pin_path.clone(), pin("new"))],
            "committed",
            stuck,
        );
        let store = WorkflowStore::project(project.path());
        let launch = plan(task_universe(&task_root));
        let refused = super::workflow_run_end_snapshot::refuse_launch(&store, &launch, true);
        if stuck {
            let error = refused.err().expect("a launch over an unsettled journal");
            assert!(
                crate::command::workflow_task_set::UnsettledPublish::is(&error),
                "the child exit would lose the pause: {error:#}"
            );
            // The snapshot's own read, should the launch recovery have
            // settled nothing: refused with its reason, never read unlocked.
            let snapshot = super::workflow_run_end_snapshot::launch_snapshot(&store, &launch, true)
                .err()
                .expect("a snapshot over an unsettled journal");
            assert!(crate::command::workflow_task_set::UnsettledPublish::is(
                &snapshot
            ));
            for text in [format!("{error:#}"), format!("{snapshot:#}")] {
                for needed in ["state: committed", "Operator remedy"] {
                    assert!(text.contains(needed), "{needed} missing: {text}");
                }
            }
            let snapshot = format!("{snapshot:#}");
            assert!(snapshot.contains("no run is started"), "{snapshot}");
            assert!(store.list_runs().unwrap().is_empty(), "a run was created");
            fix();
        }
        let snapshot = super::workflow_run_end_snapshot::refuse_launch(&store, &launch, true)
            .expect("the launch goes on once the cause is fixed");
        let run = store.create_run(launch.approval_metadata_spec()).unwrap();
        save_generated_v2_metadata(&store, &run.id, &launch, snapshot).unwrap();
        let json = metadata_json(&store, &run.id);
        assert_eq!(
            json["observer_snapshot"]["portable_acceptance_identity"]["freeze_event_id"], "new",
            "the snapshot read the set mid-publish"
        );
        assert!(!pin_path.with_extension("publish-journal").exists());
    }
}
