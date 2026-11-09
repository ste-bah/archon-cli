use super::*;
use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};
use archon_workflow::task_skeleton::TaskSkeleton;

#[test]
fn s4a_three_gates_report_identical_deliverable_claim_text() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    let prd = temp.path().join("prds/PRD-X.md");
    let path = tasks.join(TASK_SKELETON_FILE);
    let mut skeleton: TaskSkeleton =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    skeleton.tasks[0].deliverable_contracts = serde_json::from_value(serde_json::json!([
        {"kind":"report","artifact_path":"out/report.txt","min_instances":1,"implements":["AC-X-404"]}
    ])).unwrap();
    let lock_path = tasks.join(TASK_SKELETON_LOCK_FILE);
    let mut predecessor: archon_workflow::task_skeleton::TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    std::fs::remove_file(&lock_path).unwrap();
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.skeleton_digest = None;
    pin.skeleton_gate = None;
    std::fs::write(&pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();
    let prepared = crate::command::workflow_task_set::prepare_skeleton_freeze_from_candidate(
        temp.path(),
        tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        bytes,
    )
    .unwrap();
    let freeze = prepared
        .findings
        .iter()
        .find(|f| {
            f.deterministic_defect
                .as_ref()
                .is_some_and(|d| d.code == "deliverable_implements_unclaimed")
        })
        .unwrap();
    let expected = "task 'TASK-X-010' deliverable 'out/report.txt' implements 'AC-X-404', which task 'TASK-X-010' does not claim in implements; add the id to the task's implements or remove it from the deliverable";
    assert_eq!(freeze.text, expected);
    // The body gate validates pending intact predecessors, not a first draft.
    skeleton.acceptance_digest = "prior".into();
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    predecessor.acceptance_digest = "prior".into();
    predecessor.digest = archon_workflow::task_set_contract::content_digest(&bytes);
    std::fs::write(lock_path, serde_json::to_vec_pretty(&predecessor).unwrap()).unwrap();
    let set = blocking_findings(temp.path(), &LintSource::Tasks(tasks.to_path_buf()));
    let body = blocking_findings(temp.path(), &LintSource::TaskFile(task));
    assert!(set.iter().any(|text| text == expected), "{set:#?}");
    assert!(body.iter().any(|text| text == expected), "{body:#?}");
}
