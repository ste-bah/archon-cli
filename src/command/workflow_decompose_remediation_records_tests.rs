//! Records a previous build wrote keep their meaning, and the consumed
//! journal survives an interrupted append.

use super::*;

fn frozen() -> crate::command::workflow_task_set::republish::test_fixture::FrozenSet {
    crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "grep -q ready out.txt",
        true,
    )])
}

/// The whole-entry digest a previous build recorded, judgment included.
fn previous_build_digest(
    contract: &archon_workflow::task_set_contract::AcceptanceContract,
) -> String {
    archon_workflow::task_set_contract::content_digest(
        &serde_json::to_vec(&contract.acceptance[0]).unwrap(),
    )
}

fn consumed_report(run: &Path, report: serde_json::Value, consumed: &str) -> Ledger {
    let reports = run.join("decomposition/reverify/reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(
        reports.join("old-report.json"),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    std::fs::write(run.join("decomposition/reverify/consumed.jsonl"), consumed).unwrap();
    Ledger {
        report_id: "old-report".into(),
        subjects: vec!["AC-F-001".into()],
    }
}

#[test]
fn a_previous_build_report_still_names_the_flagged_bytes_it_judged() {
    let set = frozen();
    let run = tempfile::tempdir().unwrap();
    let contract = set.contract();
    let report = serde_json::json!({
        "contract_digest": "judged",
        "judged_check_digests": {"AC-F-001": previous_build_digest(&contract)},
        "checks": [{"check_id": "AC-F-001", "verdict": "needs_remediation"}],
    });
    let ledger = consumed_report(
        run.path(),
        report.clone(),
        "{\"report_id\":\"old-report\"}\n",
    );
    let (findings, skipped) = consumed_ledger_review(&set.tasks, run.path(), &ledger).unwrap();
    assert!(findings.contains_key("AC-F-001"), "{skipped:?}");
    // The same digest under this build's identity version names nothing.
    let mut versioned = report;
    versioned["judged_check_identity"] = serde_json::json!(ENTRY_IDENTITY);
    consumed_report(run.path(), versioned, "{\"report_id\":\"old-report\"}\n");
    let (findings, skipped) = consumed_ledger_review(&set.tasks, run.path(), &ledger).unwrap();
    assert!(findings.is_empty());
    assert_eq!(
        skipped,
        [
            "consumed reverify report old-report subject AC-F-001 is not re-authored: the published entry is not the one the report flagged"
        ]
    );
}

#[test]
fn a_previous_build_resolved_row_still_resolves_its_subject() {
    let set = frozen();
    let run = tempfile::tempdir().unwrap();
    let contract = set.contract();
    let path = run.path().join("decomposition/reverify/resolved.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let row = serde_json::json!({"report_id": "pending", "subjects": {"AC-F-001": previous_build_digest(&contract)}});
    std::fs::write(&path, format!("{row}\n")).unwrap();
    let ledger = Ledger {
        report_id: "pending".into(),
        subjects: vec!["AC-F-001".into()],
    };
    assert!(
        unresolved_pending_subjects(run.path(), &ledger, &contract)
            .unwrap()
            .is_empty()
    );
    // This build's rows carry the identity version their digests use.
    append_resolved(
        run.path(),
        "next",
        &std::collections::BTreeMap::from([("AC-F-001".into(), "d".into())]),
    )
    .unwrap();
    let last = std::fs::read_to_string(&path).unwrap();
    let last: serde_json::Value = serde_json::from_str(last.lines().last().unwrap()).unwrap();
    assert_eq!(last["entry_identity"], ENTRY_IDENTITY);
}

#[test]
fn a_torn_consumed_tail_is_neither_an_error_nor_merged_into_the_next_row() {
    let set = frozen();
    let run = tempfile::tempdir().unwrap();
    let contract = set.contract();
    let report = serde_json::json!({
        "contract_digest": "judged",
        "judged_check_identity": ENTRY_IDENTITY,
        "judged_check_digests": {"AC-F-001": authored_entry_digest(&contract.acceptance[0])},
        "checks": [{"check_id": "AC-F-001", "verdict": "needs_remediation"}],
    });
    let torn = "{\"report_id\":\"old-report\"}\n{\"report_id\":\"inter";
    let ledger = consumed_report(run.path(), report, torn);
    assert!(
        consumed_ledger_findings(&set.tasks, run.path(), &ledger)
            .unwrap()
            .contains_key("AC-F-001")
    );
    let next = Ledger {
        report_id: "next-report".into(),
        subjects: vec!["AC-F-001".into()],
    };
    append_consumed(run.path(), &next).unwrap();
    let text =
        std::fs::read_to_string(run.path().join("decomposition/reverify/consumed.jsonl")).unwrap();
    let rows = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2, "{text}");
    assert_eq!(rows[1]["report_id"], "next-report");
    assert!(is_consumed(run.path(), "next-report").unwrap());
    assert!(consumed_ledger_findings(&set.tasks, run.path(), &ledger).is_ok());
}
