//! Whole-set repair scope requires verified weakening evidence.
use super::*;

async fn scope_for(quotes: serde_json::Value) -> GateFinding {
    let temp = corpus();
    let mut response: serde_json::Value = serde_json::from_str(&reply(&["G-WS-001"])).unwrap();
    response["verdicts"][1]["weakening_quotes"] = quotes;
    let critic = FakeCritic::new(vec![Ok(response.to_string())]);
    evaluate(temp.path(), Ok(critic))
        .await
        .findings
        .into_iter()
        .find(|f| f.text.starts_with("obligation G-WS-001"))
        .unwrap()
}

#[tokio::test]
async fn verified_weakening_quote_routes_to_the_named_body() {
    let finding = scope_for(serde_json::json!([LOOPHOLE])).await;
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert_eq!(finding.subject, "TASK-WS-001");
    assert_eq!(finding.judged_item.as_deref(), Some("G-WS-001"));
    assert!(finding.source_path.unwrap().ends_with("TASK-WS-001.md"));
}

#[tokio::test]
async fn missing_ownership_evidence_routes_to_skeleton() {
    let finding = scope_for(serde_json::json!([])).await;
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Skeleton
    );
}

#[test]
fn weakening_quote_absent_from_named_body_routes_to_skeleton() {
    // The critic normally rejects invented quotes; check routing defensively
    // for retained standings too, rather than trusting quote presence alone.
    let verdict: archon_workflow::fidelity_audit::FidelityVerdict = serde_json::from_value(
        serde_json::json!({"obligation_id":"REQ-A", "necessarily_true":false,
            "weakest_task_id":"TASK-A", "reason":"gap", "quoted_task_text":"other body",
            "weakening_quotes":["other body"], "supporting_task_id":"", "supporting_text":""}),
    )
    .unwrap();
    let tasks = BTreeMap::from([(
        "TASK-A".into(),
        ("named body".into(), PathBuf::from("TASK-A.md")),
    )]);
    assert_eq!(
        super::super::super::fidelity_candidate_fallback::finding_scope(&verdict, &tasks),
        archon_workflow::RemediationScope::Skeleton
    );
}
