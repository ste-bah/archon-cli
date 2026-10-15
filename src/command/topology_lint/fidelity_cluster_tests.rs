use super::*;

pub(super) async fn audit_again(
    cwd: &Path,
    tasks_root: &Path,
    client: &dyn WorkflowLlmClient,
) -> FidelityAudit {
    audit(cwd, tasks_root, client, &[]).await.expect("audit")
}

/// One task claiming nine obligations is asked twice, eight and then one,
/// each reply checked against exactly the ids its batch carried.
#[tokio::test]
async fn a_large_cluster_is_asked_in_bounded_batches() {
    let temp = corpus();
    let cwd = temp.path();
    let extra: Vec<String> = (1..=7).map(|n| format!("REQ-WS-{n:03}")).collect();
    let mut prd = std::fs::read_to_string(cwd.join("tasks/PRD-WS-001.md")).unwrap();
    prd.push_str("\n## Requirements\n\n");
    for id in &extra {
        prd.push_str(&format!("- {id}: requirement {id}\n"));
    }
    std::fs::write(cwd.join("tasks/PRD-WS-001.md"), prd).unwrap();
    let task = cwd.join("tasks/PRD-WS-001/TASK-WS-001.md");
    let body = std::fs::read_to_string(&task).unwrap().replace(
        "implements: [\"G-WS-001\", \"AC-WS-001\"]",
        &format!(
            "implements: [\"G-WS-001\", \"AC-WS-001\", {}]",
            extra
                .iter()
                .map(|id| format!("\"{id}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    std::fs::write(&task, body).unwrap();
    let true_verdict = |id: &str| serde_json::json!({"obligation_id": id, "necessarily_true": true, "reason": "obliged", "supporting_task_id": "TASK-WS-001", "supporting_text": SUPPORT});
    // Batches follow the sorted claim list: AC, G, REQ-001..006 then REQ-007.
    let first: Vec<_> = ["AC-WS-001", "G-WS-001"]
        .into_iter()
        .chain(extra[..6].iter().map(String::as_str))
        .map(true_verdict)
        .collect();
    let second = vec![true_verdict("REQ-WS-007")];
    let critic = FakeCritic::new(vec![
        Ok(serde_json::json!({ "verdicts": first }).to_string()),
        Ok(serde_json::json!({ "verdicts": second }).to_string()),
    ]);
    let evaluation = evaluate(cwd, Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), 2);
    assert!(
        evaluation
            .report
            .contains("9 claimed obligation(s) in 1 cluster(s), 2 call batch(es)"),
        "{}",
        evaluation.report
    );
    assert!(
        evaluation
            .report
            .contains("REQ-WS-007: necessarily true given TASK-WS-001")
    );
    let prompts = critic.prompts.lock().unwrap();
    assert!(
        prompts[0].contains("\"id\":\"REQ-WS-006\"")
            && !prompts[0].contains("\"id\":\"REQ-WS-007\"")
    );
    assert!(
        prompts[1].contains("\"id\":\"REQ-WS-007\"")
            && !prompts[1].contains("\"id\":\"AC-WS-001\"")
    );
}

#[test]
fn the_decomposition_set_gate_carries_the_provider_and_the_body_scope() {
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev")
        .expect("catalog");
    let gate = &catalog.capabilities["task-set-lint"];
    assert_eq!(
        gate.environment_profile,
        archon_workflow::EnvironmentProfileId::FreezeProvider
    );
    assert!(
        gate.remediation_scopes
            .contains(&archon_workflow::RemediationScope::Body)
    );
    assert!(gate.timeout_secs > crate::command::workflow_task_set::judge::JUDGE_TIMEOUT_SECS);
}
