use super::*;

fn landing_reconciliation_lines(fixture: &Fixture) -> Vec<String> {
    crate::command::workflow_decompose_landing_reconcile::reconcile_task_root(
        &fixture.project,
        &fixture.project.join("AC-X-001"),
        &fixture.tasks,
        false,
    )
    .unwrap()
}

#[test]
fn landing_reconciliation_reports_a_pending_skeleton_without_manual_instructions() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let mut prd = std::fs::read(&fixture.prd).unwrap();
    prd.extend_from_slice(b"\n<!-- newer acceptance freeze -->\n");
    std::fs::write(&fixture.prd, prd).unwrap();
    freeze_acceptance(&mut fixture);

    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks);
    let pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(pin_path).unwrap()).unwrap();
    assert!(
        crate::command::workflow_skeleton_pending::is_pending(&fixture.tasks, &pin).unwrap(),
        "the landing fixture must satisfy the shared pending rule"
    );

    let lines = landing_reconciliation_lines(&fixture);
    assert_eq!(
        lines,
        vec![
            "landing reconciliation: the task skeleton waits to be frozen again after the newer acceptance freeze; nothing landed"
        ]
    );
    assert!(
        lines.iter().all(|line| !line.contains("re-run")),
        "{lines:?}"
    );

    // A pending skeleton with bytes that miss its lock remains an error.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let mut prd = std::fs::read(&fixture.prd).unwrap();
    prd.extend_from_slice(b"\n<!-- newer acceptance freeze -->\n");
    std::fs::write(&fixture.prd, prd).unwrap();
    freeze_acceptance(&mut fixture);
    let path = fixture.tasks.join(TASK_SKELETON_FILE);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b" ");
    std::fs::write(path, bytes).unwrap();

    let lines = landing_reconciliation_lines(&fixture);
    assert!(
        lines[0].contains("the frozen chain does not validate"),
        "{lines:?}"
    );
    assert!(lines[0].contains("does not verify"), "{lines:?}");

    // A complete valid chain keeps its prior summary.
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);

    let lines = landing_reconciliation_lines(&fixture);
    assert_eq!(
        lines[0],
        "landing reconciliation: would settle 0 of 2 task bodies"
    );
}
