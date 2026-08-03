use super::*;

impl LifecycleDriver {
    /// Per-task adversarial review: ONE read-only critic branch per task, over
    /// that task's diff, acceptance criteria and declared
    /// `## Adversarial Review Notes` — and nothing else.
    ///
    /// Runs inside the dependency wave, immediately after the task's own
    /// verification accepted it, so a wave-1 defect is reported in wave 1
    /// instead of after wave N. Findings are stamped with the reviewing
    /// branch's task id by `attributed_findings`, so the reviewer is never
    /// asked to remember which task it is looking at.
    ///
    /// Returns the attributed findings and records them on `evidence.review`.
    /// A stage that returns nothing returns no findings — it never fabricates
    /// acceptance.
    pub(super) async fn run_per_task_adversarial_review(
        &self,
        round_id: &str,
        task_ids: &std::collections::BTreeSet<String>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
        let items = workflow_live_v2_lifecycle_adversarial::per_task_review_items(
            &self.universe,
            task_ids,
            &[&evidence.implementation, &evidence.verification],
        );
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let result = self
            .parallel(
                &format!("adversarial-review-{round_id}"),
                serde_json::json!(&items),
                serde_json::json!({
                    // `critic` is what routes this to an adversarial reviewer:
                    // it resolves to ProviderTier::Critic, whose candidate list
                    // is sherlock-holmes -> code-reviewer -> reviewer, drawn
                    // from the AgentRegistry (plugin agents included).
                    "tier": "critic",
                    "maxParallelism": "configured",
                    "task": prompts::PER_TASK_ADVERSARIAL_REVIEW_TASK,
                }),
            )
            .await?;
        let findings =
            workflow_live_v2_lifecycle_adversarial::attributed_findings(&items, &result);
        evidence.review.push(serde_json::json!({
            "kind": "adversarial-review-task",
            "reviewRound": round_id,
            "reviewedTaskIds": task_ids,
            "findings": findings,
            "result": result,
        }));
        Ok(findings)
    }

    /// One review round: the per-task findings, then the NARROWED terminal
    /// reduce, then a structural merge.
    ///
    /// `re_review` is `None` for round 1, which reuses the findings the
    /// dependency waves already produced, and `Some(ids)` afterwards, which
    /// re-runs the per-task reviewer over exactly the tasks the previous round
    /// remediated. The merge — not the reducer — is what preserves per-task
    /// findings and what derives the status: a cross-cutting reduce returning
    /// "accepted" over outstanding per-task findings would otherwise short
    /// `review_needs_remediation` and discard every one of them.
    pub(crate) async fn run_review_round(
        &self,
        review_iteration: usize,
        re_review: Option<&std::collections::BTreeSet<String>>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let per_task_findings = match re_review {
            Some(task_ids) => {
                self.run_per_task_adversarial_review(
                    &format!("round-{review_iteration}"),
                    task_ids,
                    evidence,
                )
                .await?
            }
            None => workflow_live_v2_lifecycle_adversarial::collected_per_task_findings(
                &evidence.review,
            ),
        };
        let task = if re_review.is_some() {
            prompts::CROSS_CUTTING_RE_REVIEW_TASK
        } else {
            prompts::CROSS_CUTTING_REVIEW_TASK
        };
        let cross = self
            .reduce(
                &format!("cross-cutting-review-{review_iteration}"),
                workflow_live_v2_lifecycle_cross_cutting::cross_cutting_input(
                    &self.task_universe,
                    &per_task_findings,
                ),
                "reviewer",
                task,
            )
            .await?;
        let review =
            workflow_live_v2_lifecycle_cross_cutting::merge_review(&per_task_findings, &cross);
        evidence.review.push(serde_json::json!({
            "kind": "review",
            "reviewIteration": review_iteration,
            "result": review,
        }));
        Ok(review)
    }

    pub(super) async fn run_noop_proofs(
        &self,
        ready_noop_items: &[serde_json::Value],
        completed_ids: &std::collections::BTreeSet<String>,
        dependency_iteration: usize,
        accepted_this_wave: &mut std::collections::BTreeSet<String>,
        noop_reclassified_ids: &mut std::collections::BTreeSet<String>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
        let contract = self.contract();
        let ready_noop_items =
            workflow_live_v2_lifecycle_noop_routing::pin_noop_acceptance_criteria(
                &contract,
                ready_noop_items,
            );
        let noop_options = |task: &str| {
            serde_json::json!({
                "tier": "analysis",
                "itemKind": "noop_proof",
                "maxParallelism": "configured",
                "task": task,
            })
        };
        let noop_proof = self
            .parallel(
                &format!("noop-proof-verification-{dependency_iteration}"),
                serde_json::json!(&ready_noop_items),
                noop_options(prompts::NOOP_PROOF_VERIFICATION_TASK),
            )
            .await?;
        evidence.verification.push(serde_json::json!({
            "kind": "verified-noop",
            "dependencyIteration": dependency_iteration,
            "readyNoopItems": ready_noop_items,
            "result": noop_proof,
        }));
        let noop_outcomes =
            workflow_live_v2_lifecycle_noop_routing::enforce_noop_acceptance_criteria(
                &contract,
                &ready_noop_items,
                &support::outcomes_of(&noop_proof),
            );
        for id in support::matching_accepted_noop_ids(
            &contract,
            &ready_noop_items,
            &noop_outcomes,
        ) {
            accepted_this_wave.insert(id);
        }
        let mut failed = support::non_accepted_outcomes(&noop_outcomes);
        if !failed.is_empty() {
            let mut attempt = 1usize;
            let mut retry_items = ready_noop_items.clone();
            while !failed.is_empty() && attempt <= self.max_repair_iterations {
                let repair_id = format!("noop-evidence-repair-{dependency_iteration}-{attempt}");
                let repair = self
                    .reduce(
                        &repair_id,
                        serde_json::json!([
                            self.task_universe,
                            retry_items,
                            noop_proof,
                            failed
                        ]),
                        "reducer",
                        prompts::NOOP_EVIDENCE_REPAIR_TASK,
                    )
                    .await?;
                support::record_repair_attempt(
                    &mut evidence.repair_attempts,
                    &repair_id,
                    "evidence_repair",
                    &failed,
                    &repair,
                );
                let merged = support::merge_inventory_repair(
                    &contract,
                    &serde_json::json!({ "items": retry_items }),
                    &repair,
                );
                retry_items =
                    workflow_live_v2_lifecycle_noop_routing::pin_noop_acceptance_criteria(
                        &contract,
                        &support::array(merged.get("items")),
                    );
                if retry_items.is_empty() {
                    break;
                }
                let reverification = self
                    .parallel(
                        &format!("noop-proof-reverification-{dependency_iteration}-{attempt}"),
                        serde_json::json!(retry_items),
                        noop_options(prompts::NOOP_PROOF_REVERIFICATION_TASK),
                    )
                    .await?;
                evidence.verification.push(serde_json::json!({
                    "kind": "verified-noop-retry",
                    "dependencyIteration": dependency_iteration,
                    "noopRepairAttempt": attempt,
                    "noopRetryItems": retry_items,
                    "result": reverification,
                }));
                let reverification_outcomes =
                    workflow_live_v2_lifecycle_noop_routing::enforce_noop_acceptance_criteria(
                        &contract,
                        &retry_items,
                        &support::outcomes_of(&reverification),
                    );
                for id in support::matching_accepted_noop_ids(
                    &contract,
                    &retry_items,
                    &reverification_outcomes,
                ) {
                    accepted_this_wave.insert(id);
                }
                failed = support::non_accepted_outcomes(&reverification_outcomes);
                attempt += 1;
            }
        }
        if !failed.is_empty() {
            match workflow_live_v2_lifecycle_noop_routing::route_refuted_noops(
                &contract,
                &ready_noop_items,
                accepted_this_wave,
                &failed,
                completed_ids,
                noop_reclassified_ids,
            ) {
                workflow_live_v2_lifecycle_noop_routing::NoopProofExhaustionRoute::ScheduleImplementation(
                    items,
                ) => {
                    evidence.implementation.push(serde_json::json!({
                        "kind": "noop-proof-refutation-reclassification",
                        "dependencyIteration": dependency_iteration,
                        "canonical_task_ids": items
                            .iter()
                            .flat_map(|item| contract.canonical_ids_for(item))
                            .collect::<Vec<_>>(),
                        "items": items,
                    }));
                    return Ok(items);
                }
                workflow_live_v2_lifecycle_noop_routing::NoopProofExhaustionRoute::Block => {
                    return self
                        .final_report(
                            &format!("blocked-noop-proof-failed-{dependency_iteration}"),
                            None,
                            "needs_review",
                            serde_json::json!({
                                "taskUniverse": self.task_universe,
                                "readyNoopItems": ready_noop_items,
                                "noopProof": noop_proof,
                                "failedNoopProof": failed,
                                "noopReclassifiedTaskIds": noop_reclassified_ids,
                                "verificationEvidence": evidence.verification,
                                "repair_attempts": evidence.repair_attempts,
                            }),
                            prompts::BLOCKED_NOOP_PROOF_FAILED_TASK,
                        )
                        .await
                        .map(|()| Vec::new());
                }
            }
        }
        Ok(Vec::new())
    }

    pub(super) async fn discover_implementation_targets(
        &self,
        items: Vec<serde_json::Value>,
        discovery: &serde_json::Value,
        dependency_iteration: usize,
        noop_reclassified_ids: &std::collections::BTreeSet<String>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
        let items = support::array(Some(
            &self.with_declared_task_artifacts(serde_json::Value::Array(items)),
        ));
        let items = preserve_host_pinned_items(&self.contract(), items, noop_reclassified_ids);
        if items.iter().all(item_has_write_ownership) {
            return Ok(items);
        }

        let contract = self.contract();
        let mut inventory = contract.normalize_inventory(&serde_json::json!({ "items": items }));
        let mut attempt = 1usize;
        while support::array(inventory.get("items"))
            .iter()
            .any(|item| {
                support::work_type_for(item) == "implementation"
                    && !item_has_write_ownership(item)
            })
            && attempt <= self.max_investigation_iterations
        {
            let issues = support::array(inventory.get("items"))
                .into_iter()
                .filter(|item| {
                    support::work_type_for(item) == "implementation"
                        && !item_has_write_ownership(item)
                })
                .map(|item| {
                    serde_json::json!({
                        "kind": "target_file_discovery",
                        "field": "target_files",
                        "item_id": item.get("item_id").or_else(|| item.get("id")),
                        "canonical_task_ids": item.get("canonical_task_ids"),
                        "message": "implementation item requires repository-owned target discovery before scheduling",
                    })
                })
                .collect::<Vec<_>>();
            let call_id = format!("target-file-discovery-wave-{dependency_iteration}-{attempt}");
            let repair = self
                .reduce(
                    &call_id,
                    serde_json::json!([self.task_universe, inventory, issues, discovery]),
                    "analysis",
                    prompts::TARGET_FILE_DISCOVERY_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &call_id,
                "target_file_discovery",
                &issues,
                &repair,
            );
            inventory = contract.normalize_inventory(&support::merge_inventory_repair(
                &contract, &inventory, &repair,
            ));
            inventory = preserve_host_pinned_implementation(
                &contract,
                &inventory,
                noop_reclassified_ids,
            );
            let mut enriched_inventory = inventory.as_object().cloned().unwrap_or_default();
            enriched_inventory.insert(
                "items".to_string(),
                self.with_declared_task_artifacts(serde_json::Value::Array(support::array(
                    inventory.get("items"),
                ))),
            );
            inventory = contract.normalize_inventory(&serde_json::Value::Object(enriched_inventory));
            attempt += 1;
        }
        let items = support::array(inventory.get("items"));
        let (schedulable, unresolved): (Vec<_>, Vec<_>) =
            items.into_iter().partition(item_has_write_ownership);
        if !unresolved.is_empty() {
            evidence.implementation.push(serde_json::json!({
                "kind": "implementation-ownership-discovery-pending",
                "dependencyIteration": dependency_iteration,
                "items": unresolved,
            }));
        }
        Ok(schedulable)
    }
}
