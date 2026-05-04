//! Policy engine for governed learning.
//!
//! Inputs: a BehaviourProposal + current environment state.
//! Outputs: PolicyDecision + Vec<PolicyOutcome> (one per rule fired).
//!
//! ## Hard rules (non-negotiable)
//!
//! 1. PolicyOverride manifest changes → ALWAYS Critical → ALWAYS PendingApproval.
//! 2. HighRisk or Critical risk_level → ALWAYS PendingApproval (never AutoApplied).
//! 3. Low risk + low recent-incident-count + workspace allows auto-apply → AutoApplied.
//! 4. Otherwise → PendingApproval.
//!
//! Every policy decision is explainable: each outcome row stores rule_name,
//! evaluated inputs, outcome, and reason text. No black-box decisions.

use cozo::DbInstance;

use crate::models::*;
use crate::store;

/// Run the policy engine for a proposal against the current environment.
///
/// Returns the final PolicyDecision and a list of per-rule PolicyOutcome records.
/// Side effect: persists each PolicyOutcome row to `behaviour_policy_decisions`.
pub fn evaluate_proposal(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    allow_auto_apply: bool,
    recent_incident_count: usize,
) -> Result<(PolicyDecision, Vec<PolicyOutcome>), crate::errors::LearningError> {
    let mut outcomes = Vec::new();

    // Rule 0: PolicyOverride → ALWAYS Critical → PendingApproval
    if proposal.manifest_kind == BehaviourManifestKind::PolicyOverride {
        let outcome = record_rule(
            db,
            proposal,
            "policy_override_critical",
            PolicyDecision::PendingApproval,
            "PolicyOverride manifest changes are ALWAYS Critical and require approval",
            &serde_json::json!({
                "manifest_kind": proposal.manifest_kind.as_str(),
            }),
        )?;
        outcomes.push(outcome);
        return Ok((PolicyDecision::PendingApproval, outcomes));
    }

    // Rule 1: HighRisk or Critical → PendingApproval
    if proposal.risk_level.is_high_risk() {
        let outcome = record_rule(
            db,
            proposal,
            "high_risk_requires_approval",
            PolicyDecision::PendingApproval,
            &format!(
                "Manifest kind {} has risk level {} (High/Critical) — auto-apply denied",
                proposal.manifest_kind.as_str(),
                proposal.risk_level.as_str(),
            ),
            &serde_json::json!({
                "manifest_kind": proposal.manifest_kind.as_str(),
                "risk_level": proposal.risk_level.as_str(),
            }),
        )?;
        outcomes.push(outcome);
        return Ok((PolicyDecision::PendingApproval, outcomes));
    }

    // Rule 2: Low risk + low incidents + auto-apply allowed → AutoApplied
    if !proposal.risk_level.is_high_risk()
        && allow_auto_apply
        && recent_incident_count < 5
    {
        let outcome = record_rule(
            db,
            proposal,
            "low_risk_auto_apply",
            PolicyDecision::AutoApplied,
            &format!(
                "Low risk ({}) + {} recent incidents (< 5) + auto-apply enabled → AutoApplied",
                proposal.risk_level.as_str(),
                recent_incident_count,
            ),
            &serde_json::json!({
                "risk_level": proposal.risk_level.as_str(),
                "recent_incident_count": recent_incident_count,
                "allow_auto_apply": allow_auto_apply,
            }),
        )?;
        outcomes.push(outcome);
        return Ok((PolicyDecision::AutoApplied, outcomes));
    }

    // Rule 3: Fallback → PendingApproval
    let reason = if !allow_auto_apply {
        "Workspace does not allow auto-apply".to_string()
    } else {
        format!(
            "{} recent incidents (>= 5) — requiring approval despite low risk",
            recent_incident_count,
        )
    };
    let outcome = record_rule(
        db,
        proposal,
        "fallback_pending_approval",
        PolicyDecision::PendingApproval,
        &reason,
        &serde_json::json!({
            "risk_level": proposal.risk_level.as_str(),
            "recent_incident_count": recent_incident_count,
            "allow_auto_apply": allow_auto_apply,
        }),
    )?;
    outcomes.push(outcome);

    Ok((PolicyDecision::PendingApproval, outcomes))
}

/// Evaluate a proposal using the loaded Evidence Engine TOML policy.
pub fn evaluate_proposal_with_policy(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    policy: &archon_policy::EffectivePolicy,
    recent_incident_count: usize,
) -> Result<(PolicyDecision, Vec<PolicyOutcome>), crate::errors::LearningError> {
    let decision = policy.learning_auto_apply_decision(
        proposal.manifest_kind.as_str(),
        proposal.risk_level.as_str(),
    );
    evaluate_proposal(db, proposal, decision.allowed, recent_incident_count)
}

/// Record a single policy rule evaluation and its outcome.
fn record_rule(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    rule_name: &str,
    outcome: PolicyDecision,
    reason: &str,
    evaluated_inputs: &serde_json::Value,
) -> Result<PolicyOutcome, crate::errors::LearningError> {
    let decision_id = format!(
        "bpd-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    );
    let created_at = chrono::Utc::now().to_rfc3339();

    store::insert_policy_decision(
        db,
        &decision_id,
        &proposal.proposal_id,
        rule_name,
        &outcome,
        reason,
        evaluated_inputs,
        &created_at,
    )
    .map_err(|e| crate::errors::LearningError::Storage { message: e.to_string() })?;

    Ok(PolicyOutcome {
        rule_name: rule_name.to_string(),
        evaluated: evaluated_inputs.clone(),
        outcome,
        reason: reason.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_proposal(kind: BehaviourManifestKind, risk: RiskLevel) -> BehaviourProposal {
        BehaviourProposal {
            proposal_id: "test-prop-1".into(),
            workspace_id: "test-ws".into(),
            manifest_kind: kind,
            current_version: "v1".into(),
            proposed_version: "v2".into(),
            diff: "test diff".into(),
            evidence_ids: vec!["ev-1".into()],
            risk_level: risk,
            policy_decision: PolicyDecision::PendingApproval,
            status: ProposalStatus::Pending,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-policy-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn test_policy_override_always_pending_approval() {
        let db = test_db();
        let proposal = make_proposal(
            BehaviourManifestKind::PolicyOverride,
            RiskLevel::Critical,
        );
        let (decision, outcomes) = evaluate_proposal(&db, &proposal, true, 0).unwrap();
        assert_eq!(decision, PolicyDecision::PendingApproval);
        assert!(outcomes.iter().any(|o| o.rule_name == "policy_override_critical"));
    }

    #[test]
    fn test_high_risk_requires_approval() {
        let db = test_db();
        let proposal = make_proposal(
            BehaviourManifestKind::PromptProfile,
            RiskLevel::High,
        );
        let (decision, outcomes) = evaluate_proposal(&db, &proposal, true, 0).unwrap();
        assert_eq!(decision, PolicyDecision::PendingApproval);
        assert!(outcomes.iter().any(|o| o.rule_name == "high_risk_requires_approval"));
    }

    #[test]
    fn test_low_risk_auto_applies() {
        let db = test_db();
        let proposal = make_proposal(
            BehaviourManifestKind::RetrievalProfile,
            RiskLevel::Low,
        );
        let (decision, _outcomes) = evaluate_proposal(&db, &proposal, true, 0).unwrap();
        assert_eq!(decision, PolicyDecision::AutoApplied);
    }

    #[test]
    fn test_low_risk_high_incidents_requires_approval() {
        let db = test_db();
        let proposal = make_proposal(
            BehaviourManifestKind::RetrievalProfile,
            RiskLevel::Low,
        );
        let (decision, _outcomes) = evaluate_proposal(&db, &proposal, true, 10).unwrap();
        assert_eq!(decision, PolicyDecision::PendingApproval);
    }

    #[test]
    fn test_low_risk_auto_apply_disabled_requires_approval() {
        let db = test_db();
        let proposal = make_proposal(
            BehaviourManifestKind::RetrievalProfile,
            RiskLevel::Low,
        );
        let (decision, _outcomes) = evaluate_proposal(&db, &proposal, false, 0).unwrap();
        assert_eq!(decision, PolicyDecision::PendingApproval);
    }

    #[test]
    fn test_policy_explainability_records_outcomes() {
        let db = test_db();
        let proposal = make_proposal(
            BehaviourManifestKind::RetrievalProfile,
            RiskLevel::Low,
        );
        let (_decision, outcomes) = evaluate_proposal(&db, &proposal, true, 0).unwrap();
        assert!(!outcomes.is_empty());
        assert!(!outcomes[0].reason.is_empty());

        // Verify persistence
        let rows = store::list_policy_decisions_for_proposal(&db, "test-prop-1").unwrap();
        assert!(!rows.is_empty());
        assert_eq!(rows[0].rule_name, outcomes[0].rule_name);
    }

    #[test]
    fn test_loaded_policy_controls_auto_apply_gate() {
        let db = test_db();
        let proposal = make_proposal(
            BehaviourManifestKind::RetrievalProfile,
            RiskLevel::Low,
        );
        let denied = archon_policy::EffectivePolicy::default();
        let (decision, _) = evaluate_proposal_with_policy(&db, &proposal, &denied, 0).unwrap();
        assert_eq!(decision, PolicyDecision::PendingApproval);

        let mut allowed = archon_policy::EffectivePolicy::default();
        allowed.learning.auto_apply_low_risk = true;
        let (decision, _) = evaluate_proposal_with_policy(&db, &proposal, &allowed, 0).unwrap();
        assert_eq!(decision, PolicyDecision::AutoApplied);
    }
}
