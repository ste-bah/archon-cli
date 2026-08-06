// The terminal half of the review file: final evidence reconciliation, the
// artifact requirement, the zero-gap audit, the acceptance gate, the board
// drain, and the accepted report.
//
// Split from `review.rs` at the seam the loop already has — everything here
// runs once, after the bounded review loop has come out accepting — so neither
// half sits near the 500-line ceiling.

use super::*;

impl LifecycleDriver {
    pub(crate) async fn run_final_gates(
        &self,
        inventory: &serde_json::Value,
        artifact_inventory: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<()> {
        let mut final_iteration = 1usize;
        let reconciliation_id = format!("final-evidence-reconciliation-{final_iteration}");
        let reconciliation = self
            .reduce(
                &reconciliation_id,
                serde_json::json!([
                    self.task_universe,
                    inventory,
                    evidence.implementation,
                    evidence.verification,
                    evidence.review,
                    evidence.artifact,
                    self.governed_learning_context
                ]),
                "reducer",
                prompts::FINAL_EVIDENCE_RECONCILIATION_TASK,
            )
            .await?;
        let mut reconciliation = self
            .enforce_final_reconciliation_shape(&reconciliation_id, reconciliation, evidence)
            .await?;
        while !lifecycle_policy::boundary_repair::collection_items(&reconciliation).is_empty()
            && final_iteration <= self.max_repair_iterations
        {
            let items = lifecycle_policy::boundary_repair::collection_items(&reconciliation);
            let repair_id = format!("completion-claim-repair-{final_iteration}");
            let claim_repair = self
                .reduce(
                    &repair_id,
                    serde_json::json!(items),
                    "reducer",
                    prompts::COMPLETION_CLAIM_REPAIR_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.final_evidence_repair_attempts,
                &repair_id,
                "completion_claim_repair",
                &items,
                &claim_repair,
            );
            let artifact_checks: Vec<serde_json::Value> = {
                let candidates = {
                    let checks = support::array(claim_repair.get("artifact_checks"));
                    if checks.is_empty() {
                        support::array(claim_repair.get("items"))
                    } else {
                        checks
                    }
                };
                candidates
                    .into_iter()
                    .filter(|item| {
                        [
                            "path",
                            "artifact_path",
                            "artifactPath",
                            "artifact_id",
                            "artifactId",
                        ]
                        .iter()
                        .any(|key| support::present(item.get(*key)))
                    })
                    .collect()
            };
            if !artifact_checks.is_empty() {
                let investigation_id =
                    format!("artifact-existence-investigation-{final_iteration}");
                let investigation = self
                    .parallel(
                        &investigation_id,
                        serde_json::json!(artifact_checks),
                        serde_json::json!({
                            "tier": "analysis",
                            "task": prompts::ARTIFACT_EXISTENCE_INVESTIGATION_TASK,
                        }),
                    )
                    .await?;
                support::record_repair_attempt(
                    &mut evidence.final_evidence_repair_attempts,
                    &investigation_id,
                    "artifact_existence_investigation",
                    &artifact_checks,
                    &investigation,
                );
                evidence.artifact.push(serde_json::json!({
                    "kind": "artifact-existence-investigation",
                    "finalEvidenceIteration": final_iteration,
                    "artifactChecks": artifact_checks,
                    "result": investigation,
                }));
            }
            final_iteration += 1;
            let reconciliation_id = format!("final-evidence-reconciliation-{final_iteration}");
            let next_reconciliation = self
                .reduce(
                    &reconciliation_id,
                    serde_json::json!([
                        self.task_universe,
                        inventory,
                        evidence.implementation,
                        evidence.verification,
                        evidence.review,
                        evidence.artifact,
                        self.governed_learning_context,
                        evidence.final_evidence_repair_attempts
                    ]),
                    "reducer",
                    prompts::FINAL_EVIDENCE_RE_RECONCILIATION_TASK,
                )
                .await?;
            reconciliation = self
                .enforce_final_reconciliation_shape(
                    &reconciliation_id,
                    next_reconciliation,
                    evidence,
                )
                .await?;
        }
        if !lifecycle_policy::boundary_repair::collection_items(&reconciliation).is_empty() {
            return self
                .final_report(
                    "blocked-final-evidence-reconciliation",
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "finalEvidenceReconciliation": reconciliation,
                        "final_evidence_repair_attempts": evidence.final_evidence_repair_attempts,
                        "implementationEvidence": evidence.implementation,
                        "verificationEvidence": evidence.verification,
                        "reviewEvidence": evidence.review,
                        "artifactEvidence": evidence.artifact,
                    }),
                    prompts::BLOCKED_FINAL_EVIDENCE_RECONCILIATION_TASK,
                )
                .await;
        }

        let required_artifacts = self
            .call(
                "requireArtifact",
                "require-final-artifacts",
                Some(artifact_inventory.clone()),
                serde_json::json!({ "task": prompts::REQUIRE_FINAL_ARTIFACTS_TASK }),
            )
            .await?;
        evidence.artifact.push(serde_json::json!({
            "kind": "required-artifacts",
            "requiredArtifacts": required_artifacts,
        }));

        let final_audit = self
            .reduce(
                "final-zero-gap-audit",
                serde_json::json!([
                    self.task_universe,
                    inventory,
                    evidence.implementation,
                    evidence.verification,
                    evidence.review,
                    evidence.artifact,
                    required_artifacts,
                    self.governed_learning_context,
                    evidence.repair_attempts,
                    evidence.final_evidence_repair_attempts,
                    reconciliation
                ]),
                "reducer",
                prompts::FINAL_ZERO_GAP_AUDIT_TASK,
            )
            .await?;
        let final_gate = self
            .call(
                "qualityGate",
                "final-acceptance-gate",
                Some(serde_json::json!([
                    final_audit,
                    required_artifacts,
                    reconciliation
                ])),
                serde_json::json!({ "task": prompts::FINAL_ACCEPTANCE_GATE_TASK }),
            )
            .await?;
        let final_inputs = serde_json::json!({
            "taskUniverse": self.task_universe,
            "finalAudit": final_audit,
            "finalGate": final_gate,
            "requiredArtifacts": required_artifacts,
            "implementationEvidence": evidence.implementation,
            "verificationEvidence": evidence.verification,
            "reviewEvidence": evidence.review,
            "artifactEvidence": evidence.artifact,
            "repair_attempts": evidence.repair_attempts,
            "final_evidence_repair_attempts": evidence.final_evidence_repair_attempts,
        });
        if !support::outcome_accepted_or_noop(&final_gate) {
            return self
                .final_report(
                    "blocked-final-readiness",
                    Some(serde_json::json!([
                        final_gate,
                        final_audit,
                        required_artifacts,
                        reconciliation
                    ])),
                    "needs_review",
                    final_inputs,
                    prompts::BLOCKED_FINAL_READINESS_TASK,
                )
                .await;
        }
        // Last barrier before acceptance, and the only board read in the run.
        // Everything that could raise an item — every wave, fan-out, review
        // round and remediation pass — has completed, so the partition is total
        // and no sibling branch is still writing. Reading it here is what makes
        // the drain replay-safe; reading it inside a `Fanout` would observe
        // scheduling order and stop reproducing on resume.
        if self.block_undrained_board(evidence, &final_inputs).await? {
            return Ok(());
        }
        self.final_report(
            "final-acceptance-report",
            Some(serde_json::json!([
                final_gate,
                final_audit,
                required_artifacts,
                reconciliation
            ])),
            "accepted",
            final_inputs,
            prompts::FINAL_ACCEPTANCE_REPORT_TASK,
        )
        .await
    }
}
