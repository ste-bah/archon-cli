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
    assert_eq!(
        snapshot["canonical_task_root_identity"],
        task_root.canonicalize().unwrap().display().to_string()
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
            task_root: task_root.canonicalize().unwrap().display().to_string(),
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
