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

#[tokio::test]
async fn one_resume_skips_five_unproven_and_reauthors_seven_pending_checks() {
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
        7,
        "only pending subjects route without positive consumed-subject proof: {judged:?}"
    );
    assert_eq!(
        judged
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        pending.iter().map(|id| id.to_string()).collect()
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
