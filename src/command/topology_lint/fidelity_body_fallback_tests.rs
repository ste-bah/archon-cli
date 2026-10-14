use super::*;

#[tokio::test]
async fn whole_set_fallback_quotes_split_across_candidate_sections_block_the_body() {
    let temp = split_corpus();
    let cwd = temp.path();
    let first = "Invalid datasets may enter the backtest queue.";
    let second = "Non-native datasets are not rejected before execution.";
    let candidate = candidate_body(&format!("{first}\n\nAdditional scope:\n\n{second}"));
    let cluster = serde_json::json!({"verdicts": [{
        "obligation_id": "AC-WS-001", "necessarily_true": false,
        "weakest_task_id": "TASK-WS-002", "reason": "the set permits invalid datasets",
        "quoted_task_text": first
    }]})
    .to_string();
    let whole_set = serde_json::json!({"verdicts": [{
        "obligation_id": "AC-WS-001", "necessarily_true": false,
        "weakest_task_id": "TASK-WS-002", "reason": "the candidate permits invalid datasets",
        "quoted_task_text": first, "weakening_quotes": [first, second]
    }]})
    .to_string();
    let critic = FakeCritic::new(vec![Ok(cluster), Ok(whole_set)]);
    let (report, findings) = audit_candidate(cwd, &candidate, Ok(critic))
        .await
        .expect("audit");
    assert_eq!(findings.len(), 1, "{report}");
    assert!(findings[0].text.contains(first) && findings[0].text.contains(second));
}

#[tokio::test]
async fn empty_weakening_quotes_route_an_accurate_candidate_to_the_set_gate() {
    let temp = split_corpus();
    let cwd = temp.path();
    let claim = "Registration writes every entry to the shared registry.";
    let candidate = candidate_body(claim);
    let cluster = serde_json::json!({"verdicts": [{
        "obligation_id": "AC-WS-001", "necessarily_true": false,
        "weakest_task_id": "TASK-WS-002", "reason": "the cluster is insufficient",
        "quoted_task_text": claim
    }]})
    .to_string();
    let whole_set = serde_json::json!({"verdicts": [{
        "obligation_id": "AC-WS-001", "necessarily_true": false,
        "weakest_task_id": "TASK-WS-002", "reason": "a sibling gap remains",
        "quoted_task_text": claim, "weakening_quotes": []
    }]})
    .to_string();
    let critic = FakeCritic::new(vec![Ok(cluster), Ok(whole_set)]);
    let (report, findings) = audit_candidate(cwd, &candidate, Ok(critic))
        .await
        .expect("audit");
    assert!(findings.is_empty(), "{report}");
    assert!(report.contains("has a set-level gap"), "{report}");
}

#[tokio::test]
async fn set_gate_routes_after_landing_to_the_whole_set_weakest_task() {
    let temp = split_corpus();
    let cwd = temp.path();
    let candidate = candidate_body("Registration writes every entry to the shared registry.");
    let cluster = serde_json::json!({"verdicts": [{
        "obligation_id": "AC-WS-001", "necessarily_true": false,
        "weakest_task_id": "TASK-WS-002", "reason": "the claimant cluster has a gap",
        "quoted_task_text": "Registration writes every entry to the shared registry."
    }]})
    .to_string();
    let whole_set = serde_json::json!({"verdicts": [{
        "obligation_id": "AC-WS-001", "necessarily_true": false,
        "weakest_task_id": "TASK-WS-001", "reason": "the sibling leaves the shared store empty",
        "quoted_task_text": LOOPHOLE, "weakening_quotes": []
    }]})
    .to_string();
    let body_critic = FakeCritic::new(vec![Ok(cluster.clone()), Ok(whole_set.clone())]);
    let (body_report, body_findings) = audit_candidate(cwd, &candidate, Ok(body_critic.clone()))
        .await
        .expect("body audit");
    assert!(body_findings.is_empty(), "{body_report}");
    std::fs::write(candidate_path(cwd), candidate).expect("land candidate");

    let set_critic =
        FakeCritic::new(vec![Ok(serde_json::json!({"verdicts": [{
        "obligation_id": "G-WS-001", "necessarily_true": true,
        "weakest_task_id": "", "reason": "the task requires ingestion",
        "quoted_task_text": "", "supporting_task_id": "TASK-WS-001", "supporting_text": SUPPORT
    }]})
    .to_string()), Ok(cluster), Ok(whole_set)]);
    let evaluation = super::evaluate(cwd, Ok(set_critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    let finding = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.starts_with("obligation AC-WS-001"))
        .expect("set-level gap remains blocking");
    assert!(
        finding
            .source_path
            .as_ref()
            .unwrap()
            .ends_with("TASK-WS-001.md")
    );
    assert_eq!(
        set_critic.calls(),
        1,
        "whole-set fallback is reused by identical question"
    );
    assert!(
        set_critic.prompts.lock().unwrap()[0].contains("\"id\":\"G-WS-001\""),
        "only the unrelated uncached obligation is asked"
    );
}

#[tokio::test]
async fn recorded_false_reply_routes_the_landed_candidate_to_its_named_sibling() {
    const RECORDED_QUOTE: &str = "no task in the frozen chain is obliged to ingest live data for all 30 cells (the provider lanes TASK-DL-004…007 ingest what their own lanes' contracts name, and the coverage lane's contract is the engine, not the data)";
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path();
    let tasks = cwd.join("tasks/PRD-DL-441");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        cwd.join("tasks/PRD-DL-441.md"),
        "# Done Criteria\n\n1. Other release requirement.\n2. Another release requirement.\n3. Another release requirement.\n4. Another release requirement.\n5. Another release requirement.\n6. Another release requirement.\n7. Another release requirement.\n8. Backtests reject invalid/non-native datasets.\n",
    )
    .unwrap();
    std::fs::write(
        tasks.join("TASK-DL-008.md"),
        format!(
            "# TASK-DL-008\n\n```yaml\ntask_id: TASK-DL-008\ntitle: Sibling\ncomplexity: medium\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"UNCLAIMED-1\"]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Scope\n\n{RECORDED_QUOTE}\n"
        ),
    )
    .unwrap();
    let candidate_path = tasks.join("TASK-DL-003.md");
    let candidate_quote = "Invalid datasets may proceed without a rejection check.";
    let candidate = format!(
        "# TASK-DL-003\n\n```yaml\ntask_id: TASK-DL-003\ntitle: Candidate\ncomplexity: medium\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"DONE-8\"]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Scope\n\n{candidate_quote}\n"
    );
    let claimant_reply = serde_json::json!({"verdicts": [{
        "obligation_id": "DONE-8", "necessarily_true": false,
        "weakest_task_id": "TASK-DL-003", "reason": "the claimant allows invalid data",
        "quoted_task_text": candidate_quote, "weakening_quotes": []
    }]})
    .to_string();
    // Copied from recorded verdict 2cb937d67a983427e37c92cdcecc973fc740c3e9ebc662af20970e271fcd4c00;
    // that historical schema predates weakening_quotes, so its explicit evidence is empty.
    let recorded_whole_set_reply = serde_json::json!({"verdicts": [{
        "obligation_id": "DONE-8", "necessarily_true": false,
        "weakest_task_id": "TASK-DL-008",
        "reason": "The availability pin only proves the engine promotes cells the registry can qualify over 6 planted cells at one timeframe; no task must make all 30 available, so DONE-8's 'matrix works' can hold with 24+ unavailable cells.",
        "quoted_task_text": RECORDED_QUOTE, "weakening_quotes": []
    }]})
    .to_string();
    let body_critic = FakeCritic::new(vec![Ok(claimant_reply), Ok(recorded_whole_set_reply)]);
    let (report, findings) = audit_task_file_candidate(
        cwd,
        &candidate_path,
        &candidate,
        Ok(body_critic.clone()),
        &[],
    )
    .await
    .expect("the recorded whole-set gap is routed outside the candidate");
    assert!(findings.is_empty(), "{report}");
    std::fs::write(&candidate_path, candidate).unwrap();

    let set_critic = FakeCritic::new(Vec::new());
    let evaluation = evaluate_lint_with_fidelity(
        cwd,
        &LintSource::Tasks(tasks.clone()),
        archon_core::config::GateMode::Enforce,
        Ok(set_critic.clone()),
        &[],
    )
    .await
    .unwrap();
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    let finding = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.starts_with("obligation DONE-8"))
        .expect("set finding uses the recorded whole-set attribution");
    assert!(
        finding
            .source_path
            .as_ref()
            .unwrap()
            .ends_with("TASK-DL-008.md")
    );
    assert_eq!(
        set_critic.calls(),
        0,
        "both exact questions reuse the saved records"
    );
}
