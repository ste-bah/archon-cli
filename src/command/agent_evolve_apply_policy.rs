//! Governed apply policy for agent evolution proposals.

use anyhow::Result;
use cozo::DbInstance;

use archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord;

pub(crate) fn ensure_can_apply(
    db: &DbInstance,
    proposal: &AgentEvolutionProposalRecord,
    activate: bool,
) -> Result<()> {
    match proposal.status.as_str() {
        "rejected" => anyhow::bail!("proposal {} is rejected", proposal.proposal_id),
        "applied" => anyhow::bail!("proposal {} is already applied", proposal.proposal_id),
        _ => {}
    }

    if requires_approval(proposal) && proposal.status != "approved" {
        anyhow::bail!(
            "proposal {} requires approval before apply",
            proposal.proposal_id
        );
    }

    if activate && requires_shadow_before_activation(proposal) {
        ensure_promoting_shadow(db, proposal)?;
    }
    Ok(())
}

fn requires_approval(proposal: &AgentEvolutionProposalRecord) -> bool {
    matches!(proposal.risk_level.as_str(), "high" | "critical")
        || proposal.affects_permissions
        || proposal.affects_provider_identity
}

fn requires_shadow_before_activation(proposal: &AgentEvolutionProposalRecord) -> bool {
    matches!(proposal.risk_level.as_str(), "high" | "critical")
        || proposal.affects_permissions
        || proposal.affects_provider_identity
}

fn ensure_promoting_shadow(db: &DbInstance, proposal: &AgentEvolutionProposalRecord) -> Result<()> {
    let evaluations =
        archon_learning::agent_shadow_evaluations::list_agent_shadow_evaluations_by_proposal(
            db,
            &proposal.proposal_id,
        )?;
    let Some(latest) = evaluations.first() else {
        anyhow::bail!(
            "proposal {} requires a passing shadow evaluation before activation",
            proposal.proposal_id
        );
    };

    if latest.verdict != "promote" {
        anyhow::bail!(
            "proposal {} latest shadow evaluation verdict is {}; activation requires promote",
            proposal.proposal_id,
            latest.verdict
        );
    }
    if latest.regression_count > 0 {
        anyhow::bail!(
            "proposal {} latest shadow evaluation has {} regressions",
            proposal.proposal_id,
            latest.regression_count
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-agent-evolve-apply-policy-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    fn proposal(
        status: &str,
        risk_level: &str,
    ) -> archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord {
        archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "agent-evo-prop-1",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "prompt_profile",
            "2026-05-08T12:00:00Z",
        )
        .with_risk(risk_level, "pending_approval")
        .with_status(status)
    }

    fn insert_shadow(db: &DbInstance, verdict: &str, regression_count: i64, created_at: &str) {
        archon_learning::agent_shadow_evaluations::insert_agent_shadow_evaluation(
            db,
            &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord::new(
                format!("shadow-eval-{created_at}"),
                "agent-evo-prop-1",
                "reviewer",
                verdict,
                created_at,
            )
            .with_counts(regression_count, 3)
            .with_scores(0.61, 0.77),
        )
        .unwrap();
    }

    #[test]
    fn high_risk_apply_staging_requires_approval_not_shadow() {
        let db = test_db();
        let pending = proposal("pending", "high");
        let approved = proposal("approved", "high");

        let err = ensure_can_apply(&db, &pending, false).unwrap_err();
        assert!(err.to_string().contains("requires approval"));

        ensure_can_apply(&db, &approved, false).unwrap();
    }

    #[test]
    fn high_risk_activation_requires_latest_promoting_shadow() {
        let db = test_db();
        let approved = proposal("approved", "high");

        let err = ensure_can_apply(&db, &approved, true).unwrap_err();
        assert!(err.to_string().contains("shadow evaluation"));

        insert_shadow(&db, "promote", 0, "2026-05-08T12:00:00Z");
        ensure_can_apply(&db, &approved, true).unwrap();

        insert_shadow(&db, "needs_review", 0, "2026-05-08T12:01:00Z");
        let err = ensure_can_apply(&db, &approved, true).unwrap_err();
        assert!(err.to_string().contains("activation requires promote"));
    }

    #[test]
    fn high_risk_activation_rejects_shadow_regressions() {
        let db = test_db();
        let approved = proposal("approved", "high");

        insert_shadow(&db, "promote", 1, "2026-05-08T12:00:00Z");
        let err = ensure_can_apply(&db, &approved, true).unwrap_err();

        assert!(err.to_string().contains("regressions"));
    }

    #[test]
    fn low_risk_activation_can_skip_shadow() {
        let db = test_db();
        let pending = proposal("pending", "low");

        ensure_can_apply(&db, &pending, true).unwrap();
    }
}
