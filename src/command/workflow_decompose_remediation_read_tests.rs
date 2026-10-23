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
fn read_missing_ledger_is_none_and_corrupt_ledger_names_its_path() {
    let temp = tempfile::tempdir().unwrap();
    assert!(read(temp.path(), "missing").unwrap().is_none());

    let directory = temp.path().join("decomposition/reverify");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("remediation-corrupt.json");
    std::fs::write(&path, b"not json").unwrap();
    let error = read(temp.path(), "corrupt").unwrap_err();
    assert!(format!("{error:#}").contains(&path.display().to_string()));
}

#[test]
fn is_consumed_treats_missing_file_as_empty_and_names_invalid_utf8_path() {
    let temp = tempfile::tempdir().unwrap();
    assert!(!is_consumed(temp.path(), "report").unwrap());

    let path = temp.path().join("decomposition/reverify/consumed.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, [0xff]).unwrap();
    let error = is_consumed(temp.path(), "report").unwrap_err();
    assert!(format!("{error:#}").contains(&path.display().to_string()));
}

#[test]
fn consumed_journal_ignores_only_an_unterminated_partial_final_line() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("decomposition/reverify/consumed.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{\"report_id\":\"complete\"}\n{\"report_id\":").unwrap();

    assert!(!is_consumed(temp.path(), "partial").unwrap());
    assert!(is_consumed(temp.path(), "complete").unwrap());
}

#[test]
fn consumed_journal_rejects_a_malformed_terminated_line() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("decomposition/reverify/consumed.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{\"report_id\":\"complete\"}\nnot-json\n").unwrap();

    assert!(is_consumed(temp.path(), "missing").is_err());
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
async fn consumed_ledger_without_current_lineage_proof_skips_reauthoring_on_resume() {
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
    let reports = run_dir.path().join("decomposition/reverify/reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(
        reports.join("report-consumed.json"),
        serde_json::json!({"checks": [{"check_id": "AC-F-001", "findings": []}]}).to_string(),
    )
    .unwrap();
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
    assert_eq!(
        client.authored(),
        0,
        "missing evidence must not route a subject"
    );
}
