use super::*;

#[test]
fn self_heal_reauthors_reverted_entries_but_skips_entries_equal_to_the_repaired_lineage_version() {
    let ids = (1..=5)
        .map(|index| format!("AC-F-{index:03}"))
        .collect::<Vec<_>>();
    let checks = ids
        .iter()
        .map(|id| (id.as_str(), "grep -q ready out.txt", true))
        .collect::<Vec<_>>();
    let set =
        crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&checks);
    let contract_path = set
        .tasks
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let before_bytes = std::fs::read(&contract_path).unwrap();
    let before: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&before_bytes).unwrap();
    let mut after = before.clone();
    for entry in &mut after.acceptance {
        entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
            command: "grep -q ready out.txt && test -s out.txt".into(),
            cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
        };
    }
    let after_bytes = serde_json::to_vec_pretty(&after).unwrap();
    let before_digest = archon_workflow::task_set_contract::content_digest(&before_bytes);
    let after_digest = archon_workflow::task_set_contract::content_digest(&after_bytes);
    let pin_path = set.pin_path();
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(&pin_path);
    history.put(&before_bytes).unwrap();
    let consumed = set
        .project
        .path()
        .join(".archon/workflows/test-run/decomposition/reverify");
    std::fs::create_dir_all(&consumed).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let consumed_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    std::fs::write(
        consumed.join("consumed.jsonl"),
        serde_json::json!({"report_id":"consumed-report","consumed_at":consumed_at}).to_string(),
    )
    .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    history.put(&after_bytes).unwrap();
    // The live bytes match the transition's replaced version.
    std::fs::write(&contract_path, &before_bytes).unwrap();
    let ledger = Ledger {
        report_id: "consumed-report".into(),
        subjects: ids.clone(),
    };
    let findings = consumed_ledger_findings(set.project.path(), &set.tasks, &ledger).unwrap();
    assert_eq!(findings.len(), 5);
    for id in &ids {
        assert!(findings[id].contains("prove a reverted check"));
        assert!(findings[id].contains(&before_digest));
        assert!(findings[id].contains(&after_digest));
    }

    std::fs::write(&contract_path, &after_bytes).unwrap();
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
        consumed_ledger_findings(set.project.path(), &set.tasks, &ledger).unwrap()
    });
    assert!(findings.is_empty());
    let log = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(log.contains("subject=AC-F-001"), "{log}");
    assert!(
        log.contains("no filed pre-consumption and post-consumption repair evidence"),
        "{log}"
    );
}

#[test]
fn missing_lineage_is_not_proof_of_a_revert() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "grep -q ready out.txt",
        true,
    )]);
    let pin_path = set.pin_path();
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.lineage.clear();
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    let ledger = Ledger {
        report_id: "consumed-report".into(),
        subjects: vec!["AC-F-001".into()],
    };
    assert!(
        consumed_ledger_findings(set.project.path(), &set.tasks, &ledger)
            .unwrap()
            .is_empty()
    );
}
