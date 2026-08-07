// The review loop's exit for the third verdict.
//
// Kept out of `review.rs` only to hold the 500-line ceiling; it is one step of
// the same loop. `lifecycle_policy::assignment_invalid` decides WHETHER a
// verdict stands; this decides what the run does about it.

use serde_json::Value;

use crate::v2::lifecycle_policy::assignment_invalid;
use crate::v2::lifecycle_prompts as prompts;

use super::{LifecycleDriver, LifecycleEvidence};

impl LifecycleDriver {
    /// Stop the review loop when a per-task reviewer has established that a
    /// task should not be attempted as written.
    ///
    /// Returns `true` when it reported and the caller must return.
    ///
    /// The whole loop ends, not just that task's share of it. Remediation is
    /// run-wide — one inventory, one write fan-out, one verification gate per
    /// round — so continuing would spend the remaining rounds fixing findings
    /// against acceptance criteria that are themselves in question. Re-scoping
    /// rewrites those criteria, and nothing inside this loop is allowed to do
    /// that; escalating is how the decision reaches something that can.
    ///
    /// The report is `needs_review`, not a failure: an invalid assignment is a
    /// correct and useful thing for a reviewer to have found. It carries the
    /// invalid assignments, the tasks they name, and the round's remaining
    /// ordinary findings, so a re-scope has the same evidence the reviewer had.
    pub(crate) async fn block_assignment_invalid(
        &self,
        review_iteration: usize,
        review: &Value,
        evidence: &LifecycleEvidence,
    ) -> crate::WorkflowResult<bool> {
        let Some(admitted) = assignment_invalid::escalation(review) else {
            return Ok(false);
        };
        let invalid_task_ids = assignment_invalid::invalid_task_ids(&admitted);
        self.final_report(
            &format!("blocked-assignment-invalid-{review_iteration}"),
            None,
            "needs_review",
            serde_json::json!({
                "taskUniverse": self.task_universe,
                "review": review,
                "reviewIteration": review_iteration,
                "assignmentInvalid": admitted,
                "assignmentInvalidTaskIds": invalid_task_ids,
                "implementationEvidence": evidence.implementation,
                "verificationEvidence": evidence.verification,
                "reviewEvidence": evidence.review,
                "artifactEvidence": evidence.artifact,
                "repair_attempts": evidence.repair_attempts,
            }),
            prompts::BLOCKED_ASSIGNMENT_INVALID_TASK,
        )
        .await?;
        Ok(true)
    }
}
