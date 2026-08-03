use std::collections::BTreeSet;

use serde_json::Value;

use archon_workflow::v2::lifecycle_prompts as prompts;

use super::{LifecycleDriver, LifecycleEvidence, support};

const REVIEW_VERIFICATION_EXECUTION_RETRIES: usize = 2;

impl LifecycleDriver {
    pub(super) async fn run_review_verification_gate(
        &self,
        review_iteration: usize,
        review: &Value,
        review_fixes: &Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<bool> {
        let plan = self
            .review_verification_plan(review_iteration, review_fixes, evidence)
            .await?;
        let plan_items = support::array(plan.get("items"));
        if plan_items.is_empty() {
            self.block_empty_review_verification(
                review_iteration,
                review,
                review_fixes,
                &plan,
                evidence,
            )
            .await?;
            return Ok(false);
        }
        let items = support::split_focused_verification_items(&self.contract(), &plan_items);
        let verification = self
            .run_review_verification_with_retries(review_iteration, items, evidence)
            .await?;
        if !support::outcome_accepted_or_noop(&verification) {
            self.block_failed_review_verification(
                review_iteration,
                review_fixes,
                &verification,
                evidence,
            )
            .await?;
            return Ok(false);
        }
        Ok(true)
    }

    async fn run_review_verification_with_retries(
        &self,
        review_iteration: usize,
        mut items: Vec<Value>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Value> {
        let mut verification = self
            .run_review_verification_wave(review_iteration, 0, &items, evidence)
            .await?;
        record_review_verification(evidence, review_iteration, 0, &items, &verification);
        for retry in 1..=REVIEW_VERIFICATION_EXECUTION_RETRIES {
            if support::outcome_accepted_or_noop(&verification)
                || !review_verification_has_execution_failure(&verification)
            {
                break;
            }
            items = review_verification_execution_retry_items(&items, &verification);
            if items.is_empty() {
                break;
            }
            record_review_verification_retry(evidence, review_iteration, retry, &verification);
            verification = self
                .run_review_verification_wave(review_iteration, retry, &items, evidence)
                .await?;
            record_review_verification(evidence, review_iteration, retry, &items, &verification);
        }
        Ok(verification)
    }

    async fn review_verification_plan(
        &self,
        review_iteration: usize,
        review_fixes: &Value,
        evidence: &LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Value> {
        self.reduce(
            &format!("review-verification-plan-{review_iteration}"),
            serde_json::json!([self.task_universe, review_fixes, evidence.implementation]),
            "reducer",
            prompts::REVIEW_VERIFICATION_PLAN_TASK,
        )
        .await
    }

    async fn run_review_verification_wave(
        &self,
        review_iteration: usize,
        retry: usize,
        items: &[Value],
        evidence: &LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Value> {
        let id = if retry == 0 {
            format!("review-verification-wave-{review_iteration}")
        } else {
            format!("review-verification-wave-{review_iteration}-retry-{retry}")
        };
        let items = super::workflow_live_v2_lifecycle_verify_options::prepare_verification_items(
            items.to_vec(),
            self.project_artifact_root.as_deref(),
            &evidence.implementation,
            &self.task_universe,
        );
        self.parallel(
            &id,
            serde_json::json!(&items),
            review_verification_options(&items, prompts::REVIEW_VERIFICATION_WAVE_TASK),
        )
        .await
    }

    async fn block_empty_review_verification(
        &self,
        review_iteration: usize,
        review: &Value,
        review_fixes: &Value,
        plan: &Value,
        evidence: &LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<()> {
        self.final_report(
            &format!("blocked-empty-review-verification-{review_iteration}"),
            None,
            "needs_review",
            serde_json::json!({
                "taskUniverse": self.task_universe,
                "review": review,
                "reviewFixes": review_fixes,
                "reviewVerificationPlan": plan,
                "repair_attempts": evidence.repair_attempts,
            }),
            prompts::BLOCKED_EMPTY_REVIEW_VERIFICATION_TASK,
        )
        .await
    }

    async fn block_failed_review_verification(
        &self,
        review_iteration: usize,
        review_fixes: &Value,
        verification: &Value,
        evidence: &LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<()> {
        self.final_report(
            &format!("blocked-review-verification-failed-{review_iteration}"),
            None,
            "needs_review",
            serde_json::json!({
                "taskUniverse": self.task_universe,
                "reviewFixes": review_fixes,
                "reviewVerification": verification,
                "implementationEvidence": evidence.implementation,
                "verificationEvidence": evidence.verification,
                "repair_attempts": evidence.repair_attempts,
            }),
            prompts::BLOCKED_REVIEW_VERIFICATION_FAILED_TASK,
        )
        .await
    }
}

fn record_review_verification(
    evidence: &mut LifecycleEvidence,
    review_iteration: usize,
    retry: usize,
    items: &[Value],
    result: &Value,
) {
    evidence.verification.push(serde_json::json!({
        "kind": if retry == 0 { "review-verification" } else { "review-verification-retry" },
        "reviewIteration": review_iteration,
        "reviewVerificationRetry": retry,
        "reviewVerificationPlan": { "items": items },
        "result": result,
    }));
}

fn record_review_verification_retry(
    evidence: &mut LifecycleEvidence,
    review_iteration: usize,
    retry: usize,
    verification: &Value,
) {
    support::record_repair_attempt(
        &mut evidence.repair_attempts,
        &format!("review-verification-wave-{review_iteration}-retry-{retry}"),
        "review_verification_execution_retry",
        &support::non_accepted_outcomes(&support::outcomes_of(verification)),
        verification,
    );
}

pub(super) fn review_verification_options(items: &[Value], task: &str) -> Value {
    super::workflow_live_v2_lifecycle_verify_options::verification_options(items, task, false)
}

pub(super) fn review_verification_has_execution_failure(verification: &Value) -> bool {
    support::outcomes_of(verification)
        .iter()
        .any(outcome_is_execution_failure)
}

pub(super) fn review_verification_execution_retry_items(
    items: &[Value],
    verification: &Value,
) -> Vec<Value> {
    let failed = execution_failure_ids(verification);
    items
        .iter()
        .filter(|item| item_ids(item).iter().any(|id| failed.contains(id)))
        .cloned()
        .collect()
}

fn execution_failure_ids(verification: &Value) -> BTreeSet<String> {
    support::outcomes_of(verification)
        .into_iter()
        .filter(outcome_is_execution_failure)
        .flat_map(|outcome| item_ids(&outcome))
        .collect()
}

fn outcome_is_execution_failure(outcome: &Value) -> bool {
    if nested_string(outcome, &["failure_kind"]).as_deref() == Some("execution")
        || nested_string(outcome, &["data", "failure_kind"]).as_deref() == Some("execution")
        || nested_string(outcome, &["result", "data", "failure_kind"]).as_deref()
            == Some("execution")
    {
        return true;
    }
    let text = serde_json::to_string(outcome)
        .unwrap_or_default()
        .to_ascii_lowercase();
    text.contains("timed out") || text.contains("timeout")
}

fn nested_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_string)
}

fn item_ids(item: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["item_id", "id", "source_item_id", "split_from_item_id"] {
        if let Some(value) = item.get(key).and_then(Value::as_str) {
            ids.push(value.to_string());
        }
    }
    ids.extend(support::strings_of(item.get("source_outcome_item_ids")));
    ids
}
