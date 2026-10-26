use super::*;

async fn resume_prompt_for_report(report_id: &str, report: serde_json::Value) -> String {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
        "AC-X-001",
        "test -f build-manifest",
        true,
    )]);
    let run = tempfile::tempdir().unwrap();
    let report_dir = run.path().join("decomposition/reverify/reports");
    std::fs::create_dir_all(&report_dir).unwrap();
    std::fs::write(
        report_dir.join(format!("{report_id}.json")),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    let ledger = Ledger {
        report_id: report_id.into(),
        subjects: vec!["AC-X-001".into()],
    };
    let plan = ResumePlan {
        ledger: ledger.clone(),
        consumed: Vec::new(),
        pending: Some(ledger),
    };
    let client = crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
        |entry, _| {
            crate::command::workflow_task_set::reauthor::test_client::command_entry(
                entry,
                "test -f build-manifest && test -f build-output",
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
    client.prompts.lock().unwrap()[0].clone()
}

#[tokio::test]
async fn resume_report_seed_quotes_the_stored_pass_verdict() {
    let prompt = resume_prompt_for_report(
        "REPORT-X-001",
        serde_json::json!({
            "checks": [{"check_id":"AC-X-001", "verdict":"pass", "findings": []}]
        }),
    )
    .await;
    assert!(
        prompt.contains("The report gave check AC-X-001 the verdict \"pass\"."),
        "prompt was:\n{prompt}"
    );
}

#[tokio::test]
async fn resume_report_seed_names_a_missing_check_entry() {
    let prompt = resume_prompt_for_report("REPORT-X-002", serde_json::json!({"checks": []})).await;
    assert!(
        prompt.contains("Report REPORT-X-002 has no entry for check AC-X-001."),
        "prompt was:\n{prompt}"
    );
}

#[tokio::test]
async fn resume_report_seed_json_escapes_quotes_inside_findings() {
    let prompt = resume_prompt_for_report(
        "REPORT-X-003",
        serde_json::json!({
            "checks": [{"check_id":"AC-X-001", "findings":["the phrase \"host says pass\" was stored"]}]
        }),
    )
    .await;
    assert!(
        prompt.contains(r#""the phrase \"host says pass\" was stored""#),
        "quoted finding was not JSON-escaped; prompt was:\n{prompt}"
    );
}

#[tokio::test]
async fn resume_report_seed_reports_findings_that_are_not_text() {
    let prompt = resume_prompt_for_report(
        "REPORT-X-004",
        serde_json::json!({
            "checks": [{"check_id":"AC-X-001", "findings":[null, 7, false]}]
        }),
    )
    .await;
    assert!(
        prompt.contains("Report REPORT-X-004 has findings for check AC-X-001 that are not text and are not quoted."),
        "prompt was:\n{prompt}"
    );
}

#[tokio::test]
async fn resume_report_seed_says_no_verdict_when_it_is_absent() {
    let prompt = resume_prompt_for_report(
        "REPORT-X-005",
        serde_json::json!({
            "checks": [{"check_id":"AC-X-001", "findings": ["stored detail"]}]
        }),
    )
    .await;
    assert!(
        prompt.contains("The report gives no verdict for check AC-X-001."),
        "prompt was:\n{prompt}"
    );
}

#[tokio::test]
async fn resume_plan_attributes_findings_and_quotes_the_stored_verdict() {
    for (report_id, verdict, findings, expected_verdict) in [
        (
            "REPORT-X-001",
            Some("needs_remediation"),
            vec!["stale prose from the old tree", "another old-tree detail"],
            "The report gave check AC-X-001 the verdict \"needs_remediation\".",
        ),
        (
            "REPORT-X-002",
            None,
            vec!["recorded prose without a verdict"],
            "The report gives no verdict for check AC-X-001.",
        ),
    ] {
        let set =
            crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven(&[(
                "AC-X-001",
                "test -f build-manifest",
                true,
            )]);
        let run = tempfile::tempdir().unwrap();
        let report_dir = run.path().join("decomposition/reverify/reports");
        std::fs::create_dir_all(&report_dir).unwrap();
        let mut check = serde_json::json!({
            "check_id": "AC-X-001",
            "findings": findings,
        });
        if let Some(verdict) = verdict {
            check["verdict"] = serde_json::json!(verdict);
        }
        std::fs::write(
            report_dir.join(format!("{report_id}.json")),
            serde_json::to_vec(&serde_json::json!({
                "recorded_at": "2026-10-25T12:34:56Z",
                "checks": [check],
            }))
            .unwrap(),
        )
        .unwrap();
        let ledger = Ledger {
            report_id: report_id.into(),
            subjects: vec!["AC-X-001".into()],
        };
        let plan = ResumePlan {
            ledger: ledger.clone(),
            consumed: Vec::new(),
            pending: Some(ledger),
        };
        let client =
            crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
                |entry, _| {
                    crate::command::workflow_task_set::reauthor::test_client::command_entry(
                        entry,
                        "test -f build-manifest && test -f build-output",
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

        let prompt = client.prompts.lock().unwrap()[0].clone();
        assert!(
            prompt.contains(&format!(
                "Report {report_id} (recorded 2026-10-25T12:34:56Z) said about check AC-X-001:"
            )),
            "report attribution missing; prompt was:\n{prompt}"
        );
        for finding in findings {
            assert!(
                prompt.contains(&format!("\"{finding}\"")),
                "stored finding was not quoted; prompt was:\n{prompt}"
            );
            assert!(
                !prompt.lines().any(|line| line.trim() == finding),
                "stored finding appeared as bare prose; prompt was:\n{prompt}"
            );
        }
        assert!(prompt.contains(expected_verdict), "{prompt}");
        assert!(
            prompt.contains("It describes the tree at that time and may be out of date"),
            "{prompt}"
        );
        assert!(!prompt.contains("the host re-runs the check"), "{prompt}");
        if verdict.is_none() {
            assert!(!prompt.contains("as needing remediation"), "{prompt}");
        }
    }
}
