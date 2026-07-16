// Dependency-wave execution for the Rust decomposed-PRD lifecycle: ready-item
// selection, deadlock repair, verified-noop proofs, implementation fanout,
// remediation loops, and the ownership-expansion follow-up (body_a.js lines
// 222-500 plus the ownership splice), ported faithfully.

impl LifecycleDriver {
    pub(in super::super) async fn run_dependency_waves(
        &self,
        mut inventory: serde_json::Value,
        discovery: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let contract = self.contract();
        let (reconciled_inventory, mut noop_reclassified_ids) =
            workflow_live_v2_lifecycle_noop_routing::reclassify_inventory_contradicted_noops(
                &contract,
                &inventory,
            );
        inventory = reconciled_inventory;
        if !noop_reclassified_ids.is_empty() {
            evidence.implementation.push(serde_json::json!({
                "kind": "noop-inventory-contradiction-reclassification",
                "canonical_task_ids": noop_reclassified_ids,
            }));
        }
        let mut remaining_items = support::array(inventory.get("items"));
        let mut completed_ids = self.resume_completed_ids.clone();
        remaining_items.retain(|item| !support::item_is_completed(&contract, item, &completed_ids));
        let mut dependency_iteration = 1usize;
        let mut implementation_wave_index = 1usize;

        while !remaining_items.is_empty() && dependency_iteration <= self.max_dependency_waves {
            let ready_items = support::ready_items_from(&contract, &remaining_items, &completed_ids);
            if ready_items.is_empty() {
                let repaired = self
                    .repair_dependency_deadlock(
                        &mut inventory,
                        &mut remaining_items,
                        &completed_ids,
                        discovery,
                        dependency_iteration,
                        evidence,
                    )
                    .await?;
                if repaired {
                    continue;
                }
                return self
                    .final_report(
                        &format!("blocked-dependency-deadlock-{dependency_iteration}"),
                        None,
                        "blocked",
                        serde_json::json!({
                            "taskUniverse": self.task_universe,
                            "remainingItems": remaining_items,
                            "completed_ids": completed_ids,
                            "implementationEvidence": evidence.implementation,
                            "verificationEvidence": evidence.verification,
                            "repair_attempts": evidence.repair_attempts,
                        }),
                        prompts::BLOCKED_DEPENDENCY_DEADLOCK_TASK,
                    )
                    .await
                    .map(|()| inventory.clone());
            }

            let ready_noop_items: Vec<serde_json::Value> = ready_items
                .iter()
                .filter(|item| support::work_type_for(item) == "verified_noop")
                .cloned()
                .collect();
            let mut ready_implementation_items: Vec<serde_json::Value> = ready_items
                .iter()
                .filter(|item| support::work_type_for(item) == "implementation")
                .cloned()
                .collect();
            let mut accepted_this_wave: std::collections::BTreeSet<String> = Default::default();

            if !ready_noop_items.is_empty() {
                ready_implementation_items.extend(
                    self.run_noop_proofs(
                        &ready_noop_items,
                        dependency_iteration,
                        &mut accepted_this_wave,
                        &mut noop_reclassified_ids,
                        evidence,
                    )
                    .await?,
                );
            }

            let current_implementation_wave_index = implementation_wave_index;
            let mut implementation_candidate_ids: Vec<String> = Vec::new();
            let mut wave = serde_json::json!({ "status": "noop", "outcomes": [] });
            if !ready_implementation_items.is_empty() {
                wave = self
                    .run_implementation_wave(
                        &ready_implementation_items,
                        current_implementation_wave_index,
                        dependency_iteration,
                        &mut implementation_candidate_ids,
                        evidence,
                    )
                    .await?;
            }

            let implementation_candidate_ids_unique: Vec<String> =
                support::unique(implementation_candidate_ids)
                    .into_iter()
                    .filter(|id| !completed_ids.contains(id))
                    .collect();
            if !implementation_candidate_ids_unique.is_empty() {
                self.run_verification_lifecycle(
                    &ready_implementation_items,
                    &implementation_candidate_ids_unique,
                    current_implementation_wave_index,
                    dependency_iteration,
                    &mut accepted_this_wave,
                    evidence,
                )
                .await?;
            }

            let mut newly_completed: Vec<String> = accepted_this_wave
                .iter()
                .filter(|id| !completed_ids.contains(*id))
                .cloned()
                .collect();
            if newly_completed.is_empty() {
                newly_completed = self
                    .repair_wave_completion_evidence(
                        &ready_items,
                        &ready_noop_items,
                        &ready_implementation_items,
                        &wave,
                        &completed_ids,
                        dependency_iteration,
                        evidence,
                    )
                    .await?;
                if newly_completed.is_empty() {
                    return self
                        .final_report(
                            &format!("blocked-no-completion-{dependency_iteration}"),
                            None,
                            "needs_review",
                            serde_json::json!({
                                "taskUniverse": self.task_universe,
                                "readyItems": ready_items,
                                "readyNoopItems": ready_noop_items,
                                "readyImplementationItems": ready_implementation_items,
                                "wave": wave,
                                "implementationEvidence": evidence.implementation,
                                "verificationEvidence": evidence.verification,
                                "repair_attempts": evidence.repair_attempts,
                            }),
                            prompts::BLOCKED_NO_COMPLETION_TASK,
                        )
                        .await
                        .map(|()| inventory.clone());
                }
            }
            for id in newly_completed {
                completed_ids.insert(id);
            }
            remaining_items.retain(|item| {
                !support::item_is_completed(&contract, item, &completed_ids)
            });
            if !ready_implementation_items.is_empty() {
                implementation_wave_index += 1;
            }
            dependency_iteration += 1;
        }

        if !remaining_items.is_empty() {
            return self
                .final_report(
                    &format!("blocked-loop-exhaustion-{dependency_iteration}"),
                    None,
                    "blocked",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "remainingItems": remaining_items,
                        "completed_ids": completed_ids,
                        "implementationEvidence": evidence.implementation,
                        "verificationEvidence": evidence.verification,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_LOOP_EXHAUSTION_TASK,
                )
                .await
                .map(|()| inventory.clone());
        }
        Ok(inventory)
    }

    async fn repair_dependency_deadlock(
        &self,
        inventory: &mut serde_json::Value,
        remaining_items: &mut Vec<serde_json::Value>,
        completed_ids: &std::collections::BTreeSet<String>,
        discovery: &serde_json::Value,
        dependency_iteration: usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<bool> {
        let contract = self.contract();
        let mut attempt = 1usize;
        while attempt <= self.max_repair_iterations {
            let graph_issues = deadlock_graph_issues(&contract, remaining_items, completed_ids);
            let call_id =
                format!("dependency-graph-repair-deadlock-{dependency_iteration}-{attempt}");
            let repair = self
                .reduce(
                    &call_id,
                    serde_json::json!([
                        self.task_universe,
                        inventory,
                        remaining_items,
                        graph_issues,
                        completed_ids,
                        discovery,
                        self.governed_learning_context
                    ]),
                    "reducer",
                    prompts::DEADLOCK_GRAPH_REPAIR_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &call_id,
                "dependency_graph_repair",
                &graph_issues,
                &repair,
            );
            *inventory = contract
                .normalize_inventory(&support::merge_inventory_repair(&contract, inventory, &repair));
            *remaining_items = support::array(inventory.get("items"))
                .into_iter()
                .filter(|item| !support::item_is_completed(&contract, item, completed_ids))
                .collect();
            if !support::ready_items_from(&contract, remaining_items, completed_ids).is_empty() {
                return Ok(true);
            }
            attempt += 1;
        }
        Ok(false)
    }

    async fn run_noop_proofs(
        &self,
        ready_noop_items: &[serde_json::Value],
        dependency_iteration: usize,
        accepted_this_wave: &mut std::collections::BTreeSet<String>,
        noop_reclassified_ids: &mut std::collections::BTreeSet<String>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
        let contract = self.contract();
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
                serde_json::json!(ready_noop_items),
                noop_options(prompts::NOOP_PROOF_VERIFICATION_TASK),
            )
            .await?;
        evidence.verification.push(serde_json::json!({
            "kind": "verified-noop",
            "dependencyIteration": dependency_iteration,
            "readyNoopItems": ready_noop_items,
            "result": noop_proof,
        }));
        for id in support::matching_accepted_noop_ids(
            &contract,
            ready_noop_items,
            &support::outcomes_of(&noop_proof),
        ) {
            accepted_this_wave.insert(id);
        }
        let mut failed = support::non_accepted_outcomes(&support::outcomes_of(&noop_proof));
        if !failed.is_empty() {
            let mut attempt = 1usize;
            let mut retry_items: Vec<serde_json::Value> = ready_noop_items.to_vec();
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
                retry_items = support::array(merged.get("items"));
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
                for id in support::matching_accepted_noop_ids(
                    &contract,
                    &retry_items,
                    &support::outcomes_of(&reverification),
                ) {
                    accepted_this_wave.insert(id);
                }
                failed = support::non_accepted_outcomes(&support::outcomes_of(&reverification));
                attempt += 1;
            }
        }
        if !failed.is_empty() {
            match workflow_live_v2_lifecycle_noop_routing::route_refuted_noops(
                &contract,
                ready_noop_items,
                accepted_this_wave,
                &failed,
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

    async fn repair_wave_completion_evidence(
        &self,
        ready_items: &[serde_json::Value],
        ready_noop_items: &[serde_json::Value],
        ready_implementation_items: &[serde_json::Value],
        _wave: &serde_json::Value,
        completed_ids: &std::collections::BTreeSet<String>,
        dependency_iteration: usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Vec<String>> {
        let contract = self.contract();
        let call_id = format!("wave-completion-evidence-repair-{dependency_iteration}");
        let repair = self
            .reduce(
                &call_id,
                serde_json::json!([
                    self.task_universe,
                    ready_items,
                    ready_noop_items,
                    ready_implementation_items,
                    evidence.implementation,
                    evidence.verification
                ]),
                "reducer",
                prompts::WAVE_COMPLETION_EVIDENCE_REPAIR_TASK,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &call_id,
            "completion_evidence_repair",
            ready_items,
            &repair,
        );
        let normalized = contract.normalize_inventory(&repair);
        let outcomes = {
            let items = support::array(normalized.get("items"));
            if items.is_empty() {
                support::array(normalized.get("outcomes"))
            } else {
                items
            }
        };
        Ok(
            support::matching_accepted_completion_ids(&contract, ready_items, &outcomes)
                .into_iter()
                .filter(|id| !completed_ids.contains(id))
                .collect(),
        )
    }
}

/// JS `generatedContractInventoryGraphIssues(remainingItems, completedIds)` —
/// deadlock diagnostics passed to the repair reducer.
fn deadlock_graph_issues(
    contract: &LifecycleContract<'_>,
    remaining_items: &[serde_json::Value],
    completed_ids: &std::collections::BTreeSet<String>,
) -> Vec<serde_json::Value> {
    let mut issues = Vec::new();
    for item in remaining_items {
        let blocked_on: Vec<String> = contract
            .dependency_ids_for(item)
            .into_iter()
            .filter(|id| !completed_ids.contains(id))
            .collect();
        if blocked_on.is_empty() {
            continue;
        }
        issues.push(serde_json::json!({
            "kind": "dependency_graph_repair",
            "field": "dependency_ids",
            "message": format!("item is blocked on incomplete dependencies: {}", blocked_on.join(", ")),
            "item_id": item.get("item_id").or_else(|| item.get("id")),
            "canonical_task_ids": support::array(item.get("canonical_task_ids")),
        }));
    }
    issues
}
