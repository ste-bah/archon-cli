use serde_json::Value;

use archon_workflow::v2::lifecycle_prompts as prompts;

use super::{LifecycleDriver, LifecycleEvidence, support};

pub(super) struct ReviewRemediationBlock {
    pub id: String,
    pub inputs: Value,
}

impl LifecycleDriver {
    pub(super) async fn block_failed_review_remediation(
        &self,
        evidence: &mut LifecycleEvidence,
        review_iteration: usize,
        review: &Value,
        inventory: &Value,
        review_fixes: &Value,
    ) -> archon_workflow::WorkflowResult<bool> {
        let Some(blocked) = review_remediation_block(
            evidence,
            review_iteration,
            &self.task_universe,
            review,
            inventory,
            review_fixes,
        ) else {
            return Ok(false);
        };
        self.final_report(
            &blocked.id,
            None,
            "needs_review",
            blocked.inputs,
            prompts::BLOCKED_REVIEW_REMEDIATION_FAILED_TASK,
        )
        .await?;
        Ok(true)
    }
}

pub(super) fn review_remediation_block(
    evidence: &mut LifecycleEvidence,
    review_iteration: usize,
    task_universe: &Value,
    review: &Value,
    inventory: &Value,
    review_fixes: &Value,
) -> Option<ReviewRemediationBlock> {
    let unresolved = record_review_remediation_failure(evidence, review_iteration, review_fixes);
    if unresolved.is_empty() {
        return None;
    }
    Some(ReviewRemediationBlock {
        id: format!("blocked-review-remediation-failed-{review_iteration}"),
        inputs: blocked_review_remediation_inputs(
            task_universe,
            review,
            inventory,
            review_fixes,
            &unresolved,
            evidence,
        ),
    })
}

pub(super) fn review_remediation_failures(review_fixes: &Value) -> Vec<Value> {
    support::non_accepted_outcomes(&support::outcomes_of(review_fixes))
}

pub(super) fn record_review_remediation_failure(
    evidence: &mut LifecycleEvidence,
    review_iteration: usize,
    review_fixes: &Value,
) -> Vec<Value> {
    let unresolved = review_remediation_failures(review_fixes);
    if unresolved.is_empty() {
        return unresolved;
    }
    support::record_repair_attempt(
        &mut evidence.repair_attempts,
        &format!("review-remediation-wave-{review_iteration}"),
        "review_remediation_unresolved",
        &unresolved,
        review_fixes,
    );
    unresolved
}

pub(super) fn blocked_review_remediation_inputs(
    task_universe: &Value,
    review: &Value,
    review_remediation_inventory: &Value,
    review_fixes: &Value,
    unresolved: &[Value],
    evidence: &LifecycleEvidence,
) -> Value {
    serde_json::json!({
        "taskUniverse": task_universe,
        "review": review,
        "reviewRemediationInventory": review_remediation_inventory,
        "reviewFixes": review_fixes,
        "reviewRemediationFailures": unresolved,
        "implementationEvidence": evidence.implementation,
        "verificationEvidence": evidence.verification,
        "reviewEvidence": evidence.review,
        "artifactEvidence": evidence.artifact,
        "repair_attempts": evidence.repair_attempts,
    })
}
