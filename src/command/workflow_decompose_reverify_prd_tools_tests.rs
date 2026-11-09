use super::*;

#[test]
fn preexisting_frozen_reverify_lint_derives_tools_without_prd_input_findings() {
    // This is the task-set-lint evaluator used by reverify's execute_gate.
    // Freeze the old fixture before introducing project permissions.
    let fixture =
        fixture_with_prd(&PRD.replace("The fixture is proven.", "The `fixture` server is proven."));
    let prd_path = fixture.project.join("prds/PRD-X.md");
    let before = std::fs::read(&prd_path).unwrap();
    std::fs::write(
        fixture.project.join(".mcp.json"),
        r#"{"mcpServers":{"fixture":{"toolPolicy":{"toolPermissions":{"probe":"safe"}}}}}"#,
    )
    .unwrap();
    let evaluation = crate::command::topology_lint::evaluate_lint(
        &fixture.project,
        &crate::command::topology_lint::LintSource::Tasks(fixture.project.join("tasks/PRD-X")),
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert!(!evaluation.findings.iter().any(|finding| finding.remediation_scope == archon_workflow::RemediationScope::PrdInput));
    let tools: Vec<_> = evaluation
        .findings
        .iter()
        .filter(|finding| {
            finding
                .deterministic_defect
                .as_ref()
                .is_some_and(|identity| identity.code.starts_with("prd_tool"))
        })
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].subject, "TASK-X-010");
    assert!(tools.iter().all(|finding| matches!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body | archon_workflow::RemediationScope::Skeleton
    )));
    let output = invoke_reverify(&archon_binary(), &fixture);
    assert!(!output.stdout.is_empty() || !output.stderr.is_empty());
    let reports = fixture
        .store
        .run_dir(&fixture.run_id)
        .join("decomposition/reverify/reports");
    let path = std::fs::read_dir(reports)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let gate = report["set_gates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|gate| gate["command_id"] == "task-set-lint")
        .unwrap();
    let records = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"))
        .load_call_records()
        .unwrap();
    let record = records
        .iter()
        .find(|record| record.call.id == gate["call_id"].as_str().unwrap())
        .unwrap();
    let result: archon_workflow::HostCommandResult =
        serde_json::from_value(record.result.data.clone()).unwrap();
    let envelope = result.gate_envelope.unwrap();
    assert!(!envelope.policy_findings.iter().any(|finding| finding.remediation_scope == archon_workflow::RemediationScope::PrdInput));
    let findings: Vec<_> = envelope
        .policy_findings
        .iter()
        .filter(|finding| finding.text.contains("task TASK-X-010: PRD ids"))
        .collect();
    assert_eq!(findings.len(), 1, "{:?}", envelope.policy_findings);
    assert_eq!(findings[0].subject, "TASK-X-010");
    assert_eq!(
        findings[0].remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert_eq!(std::fs::read(prd_path).unwrap(), before);
}
