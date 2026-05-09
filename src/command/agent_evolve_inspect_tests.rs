use super::*;

fn test_db() -> DbInstance {
    let path = format!("/tmp/test-agent-evolve-inspect-{}.db", uuid::Uuid::new_v4());
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    db
}

#[test]
fn inspect_summarizes_proposal_evidence_and_shadow() {
    let db = test_db();
    archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
        &db,
        &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
            "ledger-1",
            "reviewer",
            "failed",
            "2026-05-08T12:00:00Z",
        )
        .with_model_provider("claude-sonnet-4-6", "anthropic")
        .with_provider_incident("provider-event-1")
        .with_gate_failed("sandbox:docker:failed"),
    )
    .unwrap();
    archon_learning::permission_runtime_events::insert_permission_runtime_event(
        &db,
        &archon_learning::permission_runtime_events::PermissionRuntimeEventRecord::new(
            "permission-1",
            "Bash",
            "ask",
            "denied",
            "2026-05-08T12:01:00Z",
        )
        .with_policy_context(
            Some("permission_rule_denied".to_string()),
            Some("deny_shell".to_string()),
            Some("docker".to_string()),
        ),
    )
    .unwrap();
    archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
        &db,
        &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "prop-1",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "tool_access_profile",
            "2026-05-08T12:02:00Z",
        )
        .with_risk("high", "manual_review_required")
        .with_expected_impact("Review repeated denied shell use.")
        .with_evidence("ledger-1")
        .with_evidence("permission-1")
        .with_permission_impact(),
    )
    .unwrap();
    archon_learning::agent_shadow_evaluations::insert_agent_shadow_evaluation(
        &db,
        &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord::new(
            "shadow-1",
            "prop-1",
            "reviewer",
            "needs_review",
            "2026-05-08T12:03:00Z",
        )
        .with_scores(0.4, 0.7)
        .with_counts(1, 3),
    )
    .unwrap();

    let inspection = AgentEvolutionInspection::load(&db, "prop-1").unwrap();

    assert_eq!(inspection.proposal.current_version, "agentv-1");
    assert_eq!(inspection.proposal.proposed_version, "agentv-2");
    assert_eq!(
        inspection.compatibility.anthropic_spoof_status,
        "unaffected"
    );
    assert!(inspection.compatibility.permissions_affected);
    assert!(inspection.compatibility.manual_review_required);
    assert_eq!(inspection.evidence.len(), 2);
    assert_eq!(inspection.evidence[0].source, "agent_performance_ledger");
    assert!(
        inspection.evidence[0]
            .summary
            .contains("gate=sandbox:docker:failed")
    );
    assert_eq!(inspection.evidence[1].source, "permission_runtime_events");
    assert_eq!(inspection.shadow_evaluations[0].verdict, "needs_review");
}

#[test]
fn inspect_resolves_provider_and_sandbox_event_references() {
    let db = test_db();
    archon_learning::runtime_events::insert_provider_runtime_event(
        &db,
        &archon_learning::runtime_models::ProviderRuntimeEventRecord::new(
            "provider-event-1",
            "anthropic",
            "direct",
            "request_failed",
            "warn",
            "2026-05-08T12:00:00Z",
        )
        .with_model("claude-sonnet-4-6")
        .with_profile("oauth-main")
        .with_reason("rate_limited"),
    )
    .unwrap();
    archon_learning::sandbox_runtime_events::insert_sandbox_runtime_event(
        &db,
        &archon_learning::sandbox_runtime_events::SandboxRuntimeEventRecord::new(
            "sandbox-event-1",
            "docker",
            "denied",
            "2026-05-08T12:01:00Z",
        )
        .with_tool("Bash")
        .with_policy(
            Some("sandbox_check_denied".to_string()),
            Some("sandbox-profile-docker".to_string()),
            Some("mirror".to_string()),
            Some("disabled".to_string()),
            Some("ro".to_string()),
        ),
    )
    .unwrap();
    archon_learning::permission_runtime_events::insert_permission_runtime_event(
        &db,
        &archon_learning::permission_runtime_events::PermissionRuntimeEventRecord::new(
            "permission-1",
            "Bash",
            "ask",
            "denied",
            "2026-05-08T12:01:30Z",
        )
        .with_policy_context(Some("deny_rule".to_string()), None, None),
    )
    .unwrap();
    archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
        &db,
        &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "prop-2",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "quality_gate_profile",
            "2026-05-08T12:02:00Z",
        )
        .with_evidence("provider_event:provider-event-1")
        .with_evidence("sandbox_event:sandbox-event-1")
        .with_evidence("permission_event:permission-1"),
    )
    .unwrap();

    let inspection = AgentEvolutionInspection::load(&db, "prop-2").unwrap();

    assert_eq!(inspection.evidence.len(), 3);
    assert_eq!(inspection.evidence[0].source, "provider_runtime_events");
    assert!(
        inspection.evidence[0]
            .summary
            .contains("event=request_failed")
    );
    assert!(
        inspection.evidence[0]
            .summary
            .contains("reason=rate_limited")
    );
    assert_eq!(inspection.evidence[1].source, "sandbox_runtime_events");
    assert!(inspection.evidence[1].summary.contains("decision=denied"));
    assert!(inspection.evidence[1].summary.contains("backend=docker"));
    assert_eq!(inspection.evidence[2].source, "permission_runtime_events");
    assert!(inspection.evidence[2].summary.contains("reason=deny_rule"));
}
