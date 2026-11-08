use super::*;

#[test]
fn body_check_feedback_uses_unquoted_check_id() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-1", "true", true,
    )]);
    let run = tempfile::tempdir().unwrap();
    let reports = run.path().join("decomposition/reverify/reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(
        reports.join("report.json"),
        serde_json::json!({"checks": [{
            "check_id": "AC-1", "remediation_route": "task", "findings": ["expected result absent"]
        }]})
        .to_string(),
    )
    .unwrap();
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(
        &std::fs::read(
            set.tasks
                .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        )
        .unwrap(),
    )
    .unwrap();
    let contract = serde_json::from_slice(
        &std::fs::read(
            set.tasks
                .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        )
        .unwrap(),
    )
    .unwrap();
    let id = &skeleton.tasks[0].task_id;
    let ledger = Ledger {
        report_id: "report".into(),
        subjects: vec![id.clone()],
    };
    let mut arguments = serde_json::json!({});
    body_feedback(
        run.path(),
        &mut arguments,
        Some(&ledger),
        &skeleton,
        &contract,
    )
    .unwrap();
    assert_eq!(
        arguments["remediationBodyFindings"][id][0],
        "Check AC-1 reported: expected result absent"
    );
}
