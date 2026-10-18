use super::*;

#[test]
fn read_selects_only_the_ledger_named_by_the_latest_report() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("decomposition/reverify");
    std::fs::create_dir_all(&root).unwrap();
    for (report, task) in [("old", "TASK-OLD"), ("new", "TASK-NEW")] {
        let report_id = format!("reverify-{report}-report");
        std::fs::write(
            root.join(format!("remediation-{report_id}.json")),
            serde_json::json!({
                "schema_version": 1, "report_id": report_id, "subjects": [task]
            })
            .to_string(),
        )
        .unwrap();
    }
    let ledger = read(temp.path(), "reverify-new-report").unwrap().unwrap();
    assert_eq!(ledger.report_id, "reverify-new-report");
    assert_eq!(ledger.subjects, vec!["TASK-NEW"]);
}

#[test]
fn read_deduplicates_subjects_without_changing_first_seen_order() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("decomposition/reverify");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("remediation-report.json"),
        serde_json::json!({
            "schema_version": 1,
            "report_id": "report",
            "subjects": ["TASK-X-010", "TASK-X-020", "TASK-X-010", "TASK-X-010"]
        })
        .to_string(),
    )
    .unwrap();
    let ledger = read(temp.path(), "report").unwrap().unwrap();
    assert_eq!(ledger.subjects, vec!["TASK-X-010", "TASK-X-020"]);
}

#[test]
fn generated_ledger_subjects_keep_first_seen_order() {
    let subjects = deduplicate_subjects(
        ["TASK-X-010", "TASK-X-020", "TASK-X-010", "TASK-X-010"]
            .into_iter()
            .map(str::to_owned),
    );
    assert_eq!(subjects, vec!["TASK-X-010", "TASK-X-020"]);
}

#[tokio::test]
async fn consumed_ledger_does_not_reauthor_checks_on_resume() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "jq -e '.a == true' out.json",
        true,
    )]);
    let run_dir = tempfile::tempdir().unwrap();
    let ledger = Ledger {
        report_id: "report-consumed".into(),
        subjects: vec!["AC-F-001".into()],
    };
    append_consumed(run_dir.path(), &ledger).unwrap();
    let client = crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
        |entry, _| {
            crate::command::workflow_task_set::reauthor::test_client::command_entry(
                entry,
                "jq -e '.a == true and .b == 1' out.json",
            )
        },
        |_, _| true,
    );

    reauthor_check_subjects_on_resume(
        (set.project.path(), &set.prd, &set.tasks, run_dir.path()),
        Some(&ledger),
        &client,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();
    assert_eq!(client.authored(), 0);
}
