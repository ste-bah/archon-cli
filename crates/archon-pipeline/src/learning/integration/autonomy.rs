use cozo::DbInstance;

use super::types::LearningIntegrationConfig;

pub(super) fn evaluate_and_apply_generated_proposal(
    store: &DbInstance,
    proposal: &archon_learning::models::BehaviourProposal,
    config: &LearningIntegrationConfig,
) {
    let policy = archon_learning::policy::AutonomousLearningPolicy {
        enabled: config.autonomous_behaviour_apply,
        max_risk: config.autonomous_max_risk.clone(),
        min_evidence: config.autonomous_min_evidence,
        max_recent_incidents: config.autonomous_max_recent_incidents,
    };
    let decision =
        match archon_learning::policy::evaluate_proposal_autonomous(store, proposal, &policy, 0) {
            Ok((decision, _)) => decision,
            Err(e) => {
                tracing::warn!(
                    proposal_id = %proposal.proposal_id,
                    "learning proposal autonomous evaluation failed: {e}"
                );
                return;
            }
        };
    if decision != archon_learning::models::PolicyDecision::AutoApplied {
        return;
    }
    if let Err(e) = archon_learning::apply::apply_decision(
        store,
        &proposal.proposal_id,
        decision,
        None,
        Some("learning-integration-autonomous"),
    ) {
        tracing::warn!(
            proposal_id = %proposal.proposal_id,
            "learning proposal autonomous apply failed: {e}"
        );
    }
}
