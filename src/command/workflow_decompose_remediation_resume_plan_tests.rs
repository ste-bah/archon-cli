use super::*;

#[test]
fn an_unproven_pending_subject_keeps_its_remediation_ledger_unconsumed() {
    let run = tempfile::tempdir().unwrap();
    let pending = Ledger {
        report_id: "pending-report".into(),
        subjects: vec!["AC-PENDING".into(), "TASK-OTHER".into()],
    };
    let mut plan = Some(ResumePlan {
        ledger: pending.clone(),
        consumed: Vec::new(),
        pending: Some(pending),
    });
    withhold_pending_ledger_while_unproven(
        &mut plan,
        &std::collections::BTreeMap::from([("AC-PENDING".into(), "host reason".into())]),
    );
    finish_resume(run.path(), plan, "run continues".into()).unwrap();
    assert!(!is_consumed(run.path(), "pending-report").unwrap());
}

#[test]
fn pending_subjects_are_resolved_only_while_the_published_entry_digest_matches() {
    let run = tempfile::tempdir().unwrap();
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[
        ("AC-F-001", "test -f a", true),
        ("AC-F-002", "test -f b", true),
    ]);
    let ledger = Ledger {
        report_id: "reverify-7281".into(),
        subjects: vec!["AC-F-001".into(), "AC-F-002".into()],
    };
    let contract = set.contract();
    let digest = super::entry_digest(&contract, "AC-F-001").unwrap().unwrap();
    append_resolved(
        run.path(),
        &ledger.report_id,
        &std::collections::BTreeMap::from([("AC-F-001".into(), digest)]),
    )
    .unwrap();
    assert_eq!(
        unresolved_pending_subjects(run.path(), &ledger, &contract).unwrap(),
        std::collections::BTreeSet::from(["AC-F-002".into()])
    );
    let changed = archon_workflow::task_set_contract::AcceptanceContract {
        acceptance: contract
            .acceptance
            .into_iter()
            .map(|mut entry| {
                if entry.id == "AC-F-001" {
                    entry.criterion.push_str(" changed");
                }
                entry
            })
            .collect(),
        ..contract
    };
    assert_eq!(
        unresolved_pending_subjects(run.path(), &ledger, &changed).unwrap(),
        std::collections::BTreeSet::from(["AC-F-001".into(), "AC-F-002".into()])
    );
}

#[test]
fn resolved_journal_ignores_only_an_interrupted_unterminated_final_line() {
    let run = tempfile::tempdir().unwrap();
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "test -f a",
        true,
    )]);
    let ledger = Ledger {
        report_id: "reverify-tail-report".into(),
        subjects: vec!["AC-F-001".into()],
    };
    let path = run.path().join("decomposition/reverify/resolved.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "{\"report_id\":\"other\",\"subjects\":{}}\n{\"report_id\":",
    )
    .unwrap();
    assert!(
        unresolved_pending_subjects(run.path(), &ledger, &set.contract())
            .unwrap()
            .contains("AC-F-001")
    );
    append_resolved(
        run.path(),
        &ledger.report_id,
        &std::collections::BTreeMap::from([(
            "AC-F-001".into(),
            entry_digest(&set.contract(), "AC-F-001").unwrap().unwrap(),
        )]),
    )
    .unwrap();
    assert!(
        unresolved_pending_subjects(run.path(), &ledger, &set.contract())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn resolved_journal_rejects_a_malformed_complete_line() {
    let run = tempfile::tempdir().unwrap();
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "test -f a",
        true,
    )]);
    let ledger = Ledger {
        report_id: "reverify-bad-report".into(),
        subjects: vec!["AC-F-001".into()],
    };
    let path = run.path().join("decomposition/reverify/resolved.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json\n").unwrap();
    assert!(unresolved_pending_subjects(run.path(), &ledger, &set.contract()).is_err());
}

#[tokio::test]
async fn pending_publication_in_pin_lineage_is_reconciled_without_reauthoring() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-F-001",
        "test -f a",
        true,
    )]);
    let run = tempfile::tempdir().unwrap();
    let ledger = Ledger {
        report_id: "reverify-published-report".into(),
        subjects: vec!["AC-F-001".into()],
    };
    let report_dir = run.path().join("decomposition/reverify/reports");
    std::fs::create_dir_all(&report_dir).unwrap();
    let old_bytes = set.contract_bytes();
    let old_digest = archon_workflow::task_set_contract::content_digest(&old_bytes);
    std::fs::write(
        report_dir.join(format!("{}.json", ledger.report_id)),
        serde_json::to_vec(&serde_json::json!({"contract_digest": old_digest})).unwrap(),
    )
    .unwrap();

    let contract_path = set
        .tasks
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let mut contract = set.contract();
    contract.acceptance[0].criterion.push_str(" published");
    let new_bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let new_digest = archon_workflow::task_set_contract::content_digest(&new_bytes);
    std::fs::write(&contract_path, &new_bytes).unwrap();
    let pin_path = set.pin_path();
    let mut pin = set.pin();
    let from = pin.identity();
    pin.acceptance_digest = new_digest.clone();
    let mut to = from.clone();
    to.acceptance_digest = new_digest;
    pin.lineage
        .push(archon_workflow::task_set_lineage::PinTransition::extending(
            &[],
            from,
            to,
            std::collections::BTreeSet::from(["AC-F-001".into()]),
            "test publication before journal append",
        ));
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();

    let plan = ResumePlan {
        ledger: ledger.clone(),
        consumed: Vec::new(),
        pending: Some(ledger),
    };
    let client = crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
        |entry, _| {
            crate::command::workflow_task_set::reauthor::test_client::command_entry(
                entry,
                "test -s a",
            )
        },
        |_, _| true,
    );
    reauthor_resume_plan(
        (set.project.path(), &set.prd, &set.tasks, run.path()),
        &plan,
        &client,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();
    assert!(client.judged_ids.lock().unwrap().is_empty());
    let resolved =
        std::fs::read_to_string(run.path().join("decomposition/reverify/resolved.jsonl")).unwrap();
    assert!(resolved.contains("reverify-published-report"));
}

#[tokio::test]
async fn one_resume_skips_five_unproven_and_already_resolved_pending_checks() {
    let reverted = ["AC-F-001", "AC-F-002", "AC-F-003", "AC-F-004", "AC-F-005"];
    let pending = [
        "AC-F-006", "AC-F-007", "AC-F-008", "AC-F-009", "AC-F-010", "AC-F-011", "AC-F-012",
    ];
    let ids = reverted.into_iter().chain(pending).collect::<Vec<_>>();
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(
        &ids.iter()
            .map(|id| (*id, "test -f prds/PRD-F.md", true))
            .collect::<Vec<_>>(),
    );
    let run = tempfile::tempdir().unwrap();
    let report_dir = run.path().join("decomposition/reverify/reports");
    std::fs::create_dir_all(&report_dir).unwrap();
    std::fs::write(
        report_dir.join("reverify-7281-report.json"),
        serde_json::to_vec(&serde_json::json!({
            "checks": pending.iter().map(|id| serde_json::json!({"check_id":id,"findings":["pending repair finding"]})).collect::<Vec<_>>()
        })).unwrap(),
    ).unwrap();
    let consumed_path = run.path().join("decomposition/reverify/consumed.jsonl");
    std::fs::write(&consumed_path, "{\"report_id\":\"reverify-7126-report\"}\n").unwrap();
    let consumed = Ledger {
        report_id: "reverify-7126-report".into(),
        subjects: reverted.iter().map(|id| id.to_string()).collect(),
    };
    let pending_ledger = Ledger {
        report_id: "reverify-7281-report".into(),
        subjects: pending.iter().map(|id| id.to_string()).collect(),
    };
    let published_contract = set.contract();
    let resolved_digest = entry_digest(&published_contract, pending[0])
        .unwrap()
        .unwrap();
    append_resolved(
        run.path(),
        &pending_ledger.report_id,
        &std::collections::BTreeMap::from([(pending[0].to_string(), resolved_digest)]),
    )
    .unwrap();
    let plan = ResumePlan {
        ledger: Ledger {
            report_id: pending_ledger.report_id.clone(),
            subjects: ids.iter().map(|id| id.to_string()).collect(),
        },
        consumed: vec![consumed],
        pending: Some(pending_ledger),
    };
    let client = crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
        |entry, _| {
            crate::command::workflow_task_set::reauthor::test_client::command_entry(
                entry,
                "test -s prds/PRD-F.md",
            )
        },
        |_, _| true,
    );
    reauthor_resume_plan(
        (set.project.path(), &set.prd, &set.tasks, run.path()),
        &plan,
        &client,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();
    let judged = client.judged_ids.lock().unwrap().clone();
    assert_eq!(
        judged.len(),
        6,
        "a pending subject whose published entry digest is unchanged is not re-authored: {judged:?}"
    );
    assert_eq!(
        judged
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        pending.iter().skip(1).map(|id| id.to_string()).collect()
    );
    finish_resume(run.path(), Some(plan), "done".into()).unwrap();
    let report_ids = std::fs::read_to_string(consumed_path)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["report_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        report_ids,
        vec!["reverify-7126-report", "reverify-7281-report"]
    );
}
