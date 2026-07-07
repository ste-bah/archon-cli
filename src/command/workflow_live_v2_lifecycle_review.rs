// Adversarial review and final acceptance gates for the Rust decomposed-PRD
// lifecycle — ported faithfully from
// workflow_live_generated_scaffold_body_b.js (artifact inventory, bounded
// review remediation loop with hard cap 6, final evidence reconciliation,
// requireArtifact, zero-gap audit, quality gate, final report).

impl LifecycleDriver {
    pub(in super::super) async fn run_review_and_final_gates(
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
        let mut review = self
            .reduce(
                &format!("adversarial-review-{review_iteration}"),
                serde_json::json!([
                    self.task_universe,
                    evidence.implementation,
                    evidence.verification,
                    evidence.artifact,
                    self.governed_learning_context
                ]),
                "reviewer",
                prompts::ADVERSARIAL_REVIEW_TASK,
            )
            .await?;
        evidence.review.push(serde_json::json!({
            "kind": "review",
            "reviewIteration": review_iteration,
            "result": review,
        }));

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
                review_remediation_inventory =
                    remediation::normalize_review_remediation_inventory(&contract, &repair);
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
            let review_verification_plan = self
                .reduce(
                    &format!("review-verification-plan-{review_iteration}"),
                    serde_json::json!([
                        self.task_universe,
                        review_fixes,
                        evidence.implementation
                    ]),
                    "reducer",
                    prompts::REVIEW_VERIFICATION_PLAN_TASK,
                )
                .await?;
            let plan_items = support::array(review_verification_plan.get("items"));
            if plan_items.is_empty() {
                return self
                    .final_report(
                        &format!("blocked-empty-review-verification-{review_iteration}"),
                        None,
                        "needs_review",
                        serde_json::json!({
                            "taskUniverse": self.task_universe,
                            "review": review,
                            "reviewFixes": review_fixes,
                            "reviewVerificationPlan": review_verification_plan,
                            "repair_attempts": evidence.repair_attempts,
                        }),
                        prompts::BLOCKED_EMPTY_REVIEW_VERIFICATION_TASK,
                    )
                    .await;
            }
            let split_items = support::split_focused_verification_items(&contract, &plan_items);
            let review_verification = self
                .parallel(
                    &format!("review-verification-wave-{review_iteration}"),
                    serde_json::json!(split_items),
                    serde_json::json!({
                        "tier": "coder",
                        "task": prompts::REVIEW_VERIFICATION_WAVE_TASK,
                    }),
                )
                .await?;
            evidence.verification.push(serde_json::json!({
                "kind": "review-verification",
                "reviewIteration": review_iteration,
                "reviewVerificationPlan": { "items": split_items },
                "result": review_verification,
            }));
            if !support::outcome_accepted_or_noop(&review_verification) {
                return self
                    .final_report(
                        &format!("blocked-review-verification-failed-{review_iteration}"),
                        None,
                        "needs_review",
                        serde_json::json!({
                            "taskUniverse": self.task_universe,
                            "reviewFixes": review_fixes,
                            "reviewVerification": review_verification,
                            "implementationEvidence": evidence.implementation,
                            "verificationEvidence": evidence.verification,
                            "repair_attempts": evidence.repair_attempts,
                        }),
                        prompts::BLOCKED_REVIEW_VERIFICATION_FAILED_TASK,
                    )
                    .await;
            }
            review_iteration += 1;
            review = self
                .reduce(
                    &format!("adversarial-review-{review_iteration}"),
                    serde_json::json!([
                        self.task_universe,
                        evidence.implementation,
                        evidence.verification,
                        evidence.review,
                        evidence.artifact,
                        self.governed_learning_context
                    ]),
                    "reviewer",
                    prompts::ADVERSARIAL_RE_REVIEW_TASK,
                )
                .await?;
            evidence.review.push(serde_json::json!({
                "kind": "review",
                "reviewIteration": review_iteration,
                "result": review,
            }));
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

    async fn run_final_gates(
        &self,
        inventory: &serde_json::Value,
        artifact_inventory: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<()> {
        let mut final_iteration = 1usize;
        let mut reconciliation = self
            .reduce(
                &format!("final-evidence-reconciliation-{final_iteration}"),
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
        while !support::array(reconciliation.get("items")).is_empty()
            && final_iteration <= self.max_repair_iterations
        {
            let items = support::array(reconciliation.get("items"));
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
            reconciliation = self
                .reduce(
                    &format!("final-evidence-reconciliation-{final_iteration}"),
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
        }
        if !support::array(reconciliation.get("items")).is_empty() {
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
