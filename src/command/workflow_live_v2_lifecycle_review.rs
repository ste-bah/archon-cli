use super::*;

impl LifecycleDriver {
    pub(in super::super::super) async fn run_review_and_final_gates(
        &self,
        inventory: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<()> {
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
            let remediated = workflow_live_v2_lifecycle_cross_cutting::remediated_task_ids(
                &review_remediation_inventory,
            );
            review = self
                .run_review_round(review_iteration, Some(&remediated), evidence)
                .await?;
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

    pub(super) async fn run_final_gates(
        &self,
        inventory: &serde_json::Value,
        artifact_inventory: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<()> {
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
        while !workflow_live_v2_lifecycle_boundary_repair::collection_items(&reconciliation)
            .is_empty()
            && final_iteration <= self.max_repair_iterations
        {
            let items =
                workflow_live_v2_lifecycle_boundary_repair::collection_items(&reconciliation);
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
        if !workflow_live_v2_lifecycle_boundary_repair::collection_items(&reconciliation).is_empty()
        {
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
