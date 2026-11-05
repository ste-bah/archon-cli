// The bounded review loop. Its terminal half — final reconciliation, the
// acceptance gate, the board drain and the accepted report — is `final_gates`,
// split off at the seam the loop already has rather than at a line count.

use super::*;

impl LifecycleDriver {
    pub(crate) async fn run_review_and_final_gates(
        &self,
        inventory: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<()> {
        let contract = self.contract();
        let artifact_inventory = self
            .reduce(
                "artifact-inventory",
                serde_json::json!([
                    self.task_universe,
                    inventory,
                    evidence.implementation,
                    evidence.verification
                ]),
                "reducer",
                prompts::ARTIFACT_INVENTORY_TASK,
            )
            .await?;
        let saved_artifact_inventory = self
            .call(
                "saveArtifact",
                "save-artifact-inventory",
                Some(artifact_inventory.clone()),
                serde_json::json!({ "task": prompts::SAVE_ARTIFACT_INVENTORY_TASK }),
            )
            .await?;
        evidence.artifact.push(serde_json::json!({
            "kind": "artifact-inventory",
            "artifactInventory": artifact_inventory,
            "savedArtifactInventory": saved_artifact_inventory,
        }));

        let mut review_iteration = 1usize;
        // Round 1 reuses the per-task findings the dependency waves already
        // produced (`None` = do not re-review), so the diamond costs nothing
        // extra here.
        let mut review = self
            .run_review_round(review_iteration, None, evidence)
            .await?;
        // The third verdict is checked after EVERY round, before remediation is
        // even planned. An `assignment_invalid` review still carries findings,
        // so `review_needs_remediation` would let the loop in and spend a full
        // round — and then five more — remediating a task that should not be
        // attempted at all.
        if self
            .block_assignment_invalid(review_iteration, &review, evidence)
            .await?
        {
            return Ok(());
        }

        while remediation::review_needs_remediation(&review) && review_iteration <= 6 {
            let raw_inventory = self
                .reduce(
                    &format!("review-remediation-inventory-{review_iteration}"),
                    remediation::review_remediation_input(&review),
                    "reducer",
                    prompts::REVIEW_REMEDIATION_INVENTORY_TASK,
                )
                .await?;
            let mut review_remediation_inventory =
                remediation::normalize_review_remediation_inventory(&contract, &raw_inventory);
            let mut repair_attempt = 1usize;
            while !support::array(review_remediation_inventory.get("unresolved_issues")).is_empty()
                && repair_attempt <= self.max_repair_iterations
            {
                let call_id = format!(
                    "review-remediation-inventory-repair-{review_iteration}-{repair_attempt}"
                );
                let issues = support::array(review_remediation_inventory.get("unresolved_issues"));
                let repair = self
                    .reduce(
                        &call_id,
                        serde_json::json!([
                            self.task_universe,
                            review,
                            review_remediation_inventory,
                            issues,
                            evidence.implementation,
                            evidence.verification,
                            evidence.artifact
                        ]),
                        "reducer",
                        prompts::REVIEW_REMEDIATION_INVENTORY_REPAIR_TASK,
                    )
                    .await?;
                support::record_repair_attempt(
                    &mut evidence.repair_attempts,
                    &call_id,
                    "review_remediation_shape_repair",
                    &issues,
                    &repair,
                );
                let candidate =
                    remediation::normalize_review_remediation_inventory(&contract, &repair);
                // D74: adopt the repaired inventory only when it preserves the
                // semantic identity of the items being reshaped; otherwise the
                // violations feed the next bounded attempt as issues.
                let preservation = semantic_preservation::check_items(
                    &support::array(review_remediation_inventory.get("items")),
                    &support::array(candidate.get("items")),
                );
                if preservation.passed() {
                    review_remediation_inventory = candidate;
                } else {
                    support::record_repair_attempt(
                        &mut evidence.repair_attempts,
                        &call_id,
                        "semantic_preservation_rejected",
                        &semantic_preservation::violation_issues(&preservation.violations),
                        &candidate,
                    );
                    self.record_preservation_rejection(&call_id, &preservation.violations)
                        .await?;
                    semantic_preservation::append_preservation_issues(
                        &mut review_remediation_inventory,
                        &preservation.violations,
                    );
                }
                repair_attempt += 1;
            }
            if support::array(review_remediation_inventory.get("items")).is_empty()
                || !support::array(review_remediation_inventory.get("unresolved_issues")).is_empty()
            {
                return self
                    .final_report(
                        &format!("blocked-empty-review-remediation-{review_iteration}"),
                        None,
                        "needs_review",
                        serde_json::json!({
                            "taskUniverse": self.task_universe,
                            "review": review,
                            "reviewRemediationInventory": review_remediation_inventory,
                            "reviewEvidence": evidence.review,
                            "repair_attempts": evidence.repair_attempts,
                        }),
                        prompts::BLOCKED_EMPTY_REVIEW_REMEDIATION_TASK,
                    )
                    .await;
            }
            let review_fixes = self
                .write_fanout(
                    &format!("review-remediation-wave-{review_iteration}"),
                    serde_json::json!(support::array(review_remediation_inventory.get("items"))),
                    prompts::REVIEW_REMEDIATION_WAVE_TASK,
                )
                .await?;
            evidence.implementation.push(serde_json::json!({
                "kind": "review-remediation",
                "reviewIteration": review_iteration,
                "reviewRemediationInventory": review_remediation_inventory,
                "result": review_fixes,
            }));
            if self
                .block_failed_review_remediation(
                    evidence,
                    review_iteration,
                    &review,
                    &review_remediation_inventory,
                    &review_fixes,
                )
                .await?
            {
                return Ok(());
            }
            if !self
                .run_review_verification_gate(review_iteration, &review, &review_fixes, evidence)
                .await?
            {
                return Ok(());
            }
            review_iteration += 1;
            // Re-review ONLY the tasks this round remediated. Carrying the
            // previous round's per-task findings forward unchanged would make
            // every round report the same findings until the iteration cap,
            // regardless of whether the fixes landed; re-running the per-task
            // reviewer is what makes the loop converge on evidence.
            let remediated =
                lifecycle_policy::cross_cutting::remediated_task_ids(&review_remediation_inventory);
            review = self
                .run_review_round(review_iteration, Some(&remediated), evidence)
                .await?;
            if self
                .block_assignment_invalid(review_iteration, &review, evidence)
                .await?
            {
                return Ok(());
            }
        }

        if remediation::review_needs_remediation(&review) {
            return self
                .final_report(
                    "blocked-review-unresolved",
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "review": review,
                        "implementationEvidence": evidence.implementation,
                        "verificationEvidence": evidence.verification,
                        "reviewEvidence": evidence.review,
                        "artifactEvidence": evidence.artifact,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_REVIEW_UNRESOLVED_TASK,
                )
                .await;
        }
        if !support::outcome_accepted_or_noop(&review) {
            return self
                .final_report(
                    "blocked-review-not-accepted",
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "review": review,
                        "implementationEvidence": evidence.implementation,
                        "verificationEvidence": evidence.verification,
                        "reviewEvidence": evidence.review,
                        "artifactEvidence": evidence.artifact,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_REVIEW_NOT_ACCEPTED_TASK,
                )
                .await;
        }

        self.run_final_gates(inventory, &artifact_inventory, evidence)
            .await
    }
}
