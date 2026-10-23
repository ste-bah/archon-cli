use super::*;

fn setup() -> (
    crate::command::workflow_task_set::republish::test_fixture::FrozenSet,
    tempfile::TempDir,
    Ledger,
    String,
) {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "grep -q ready out.txt",
        true,
    )]);
    let run = tempfile::tempdir().unwrap();
    let report_id = "reverify-same-id-report";
    let report_dir = run.path().join("decomposition/reverify/reports");
    std::fs::create_dir_all(&report_dir).unwrap();
    let contract_bytes = std::fs::read(
        set.tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
    )
    .unwrap();
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&contract_bytes).unwrap();
    let entry = &contract.acceptance[0];
    let entry_digest = archon_workflow::task_set_contract::content_digest(
        &serde_json::to_vec(&serde_json::json!({
            "id": &entry.id, "criterion": &entry.criterion, "check": &entry.check,
            "covers": &entry.covers, "gap_permitted": entry.gap_permitted,
        }))
        .unwrap(),
    );
    let report = serde_json::json!({
        "id": report_id,
        "contract_digest": archon_workflow::task_set_contract::content_digest(&contract_bytes),
        "judged_check_digests": { "AC-F-001": entry_digest },
        "checks": [{"check_id":"AC-F-001", "verdict":"needs_remediation", "findings":["failed"]}]
    });
    std::fs::write(
        report_dir.join(format!("{report_id}.json")),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    let consumed_path = run.path().join("decomposition/reverify/consumed.jsonl");
    std::fs::create_dir_all(consumed_path.parent().unwrap()).unwrap();
    std::fs::write(
        consumed_path,
        format!("{{\"report_id\":\"{report_id}\"}}\n"),
    )
    .unwrap();
    (
        set,
        run,
        Ledger {
            report_id: report_id.into(),
            subjects: vec!["AC-F-001".into()],
        },
        entry_digest,
    )
}

#[test]
fn flagged_authored_bytes_back_are_routed_but_repaired_bytes_are_not() {
    let (set, run, ledger, _) = setup();
    let findings = consumed_ledger_findings(&set.tasks, run.path(), &ledger).unwrap();
    assert!(findings.contains_key("AC-F-001"));

    let contract_path = set
        .tasks
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let mut contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    contract.acceptance[0].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "grep -q ready out.txt && test -s out.txt".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    std::fs::write(
        &contract_path,
        serde_json::to_vec_pretty(&contract).unwrap(),
    )
    .unwrap();
    assert!(
        consumed_ledger_findings(&set.tasks, run.path(), &ledger)
            .unwrap()
            .is_empty()
    );
    drop(run);
}

#[test]
fn judgment_only_change_preserves_authored_entry_identity() {
    let (set, run, ledger, _) = setup();
    let contract_path = set
        .tasks
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let mut contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    contract.acceptance[0].judgment.reason = "a later judgment".into();
    std::fs::write(
        &contract_path,
        serde_json::to_vec_pretty(&contract).unwrap(),
    )
    .unwrap();
    assert!(
        consumed_ledger_findings(&set.tasks, run.path(), &ledger)
            .unwrap()
            .contains_key("AC-F-001")
    );
}

#[test]
fn missing_consumed_lineage_is_empty_but_corrupt_report_names_its_path() {
    let (set, run, ledger, _) = setup();
    let consumed_path = run.path().join("decomposition/reverify/consumed.jsonl");
    std::fs::remove_file(&consumed_path).unwrap();
    assert!(
        consumed_ledger_findings(&set.tasks, run.path(), &ledger)
            .unwrap()
            .is_empty()
    );

    std::fs::write(
        &consumed_path,
        format!("{{\"report_id\":\"{}\"}}\n", ledger.report_id),
    )
    .unwrap();
    let report_path = run.path().join(format!(
        "decomposition/reverify/reports/{}.json",
        ledger.report_id
    ));
    std::fs::write(&report_path, b"not json").unwrap();
    let error = consumed_ledger_findings(&set.tasks, run.path(), &ledger).unwrap_err();
    assert!(format!("{error:#}").contains(&report_path.display().to_string()));
}

#[test]
fn legacy_report_without_digests_is_not_routed_and_logs_once() {
    let (set, run, ledger, _) = setup();
    let report_path = run
        .path()
        .join("decomposition/reverify/reports/reverify-same-id-report.json");
    std::fs::write(
        report_path,
        r#"{"checks":[{"check_id":"AC-F-001","verdict":"needs_remediation"}]}"#,
    )
    .unwrap();
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = output.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_writer(move || Capture(writer.clone()))
        .finish();
    let findings = tracing::subscriber::with_default(subscriber, || {
        consumed_ledger_findings(&set.tasks, run.path(), &ledger).unwrap()
    });
    assert!(findings.is_empty());
    let log = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert_eq!(
        log.matches("heal needs the next reverify").count(),
        1,
        "{log}"
    );
    drop(run);
}

#[test]
fn same_report_id_in_another_run_is_ignored() {
    let (set, report_root, ledger, _) = setup();
    let run_dir = set.project.path().join(".archon/workflows/current-run");
    let other_run = set.project.path().join(".archon/workflows/other-run");
    let current_consumed = run_dir.join("decomposition/reverify");
    std::fs::create_dir_all(&current_consumed).unwrap();
    std::fs::write(
        current_consumed.join("consumed.jsonl"),
        "{\"report_id\":\"another-report\"}\n",
    )
    .unwrap();
    let other_consumed = other_run.join("decomposition/reverify");
    std::fs::create_dir_all(&other_consumed).unwrap();
    std::fs::write(
        other_consumed.join("consumed.jsonl"),
        format!("{{\"report_id\":\"{}\"}}\n", ledger.report_id),
    )
    .unwrap();
    assert!(
        consumed_ledger_findings(&set.tasks, &run_dir, &ledger)
            .unwrap()
            .is_empty()
    );
    drop(report_root);
}
