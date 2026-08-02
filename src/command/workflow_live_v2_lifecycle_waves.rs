// Dependency-wave execution for the Rust decomposed-PRD lifecycle: ready-item
// selection, deadlock repair, verified-noop proofs, implementation fanout,
// remediation loops, and the ownership-expansion follow-up (body_a.js lines
// 222-500 plus the ownership splice), ported faithfully.

use super::*;

include!("workflow_live_v2_lifecycle_wave_phases.rs");

impl LifecycleDriver {
    pub(in super::super::super) async fn run_dependency_waves(
        &self,
        mut inventory: serde_json::Value,
        discovery: &serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let contract = self.contract();
        let pending_implementation_items = {
            let mut state = self
                .runtime_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut state.pending_implementation_items)
        };
        inventory =
            workflow_live_v2_lifecycle_terminal_gate::apply_pending_implementation_items(
                &contract,
                &inventory,
                pending_implementation_items,
            );
        let (reconciled_inventory, mut noop_reclassified_ids) =
            workflow_live_v2_lifecycle_noop_routing::reclassify_inventory_contradicted_noops(
                &contract,
                &inventory,
            );
        inventory = reconciled_inventory;
        {
            let mut state = self
                .runtime_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            noop_reclassified_ids.extend(state.noop_reclassified_ids.iter().cloned());
            state
                .noop_reclassified_ids
                .extend(noop_reclassified_ids.iter().cloned());
        }
        if !noop_reclassified_ids.is_empty() {
            evidence.implementation.push(serde_json::json!({
                "kind": "noop-inventory-contradiction-reclassification",
                "canonical_task_ids": noop_reclassified_ids,
            }));
        }
        let mut remaining_items = support::array(inventory.get("items"));
        let mut completed_ids = self
            .runtime_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .completed_ids
            .clone();
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
                    inventory = preserve_host_pinned_implementation(
                        &contract,
                        &inventory,
                        &noop_reclassified_ids,
                    );
                    remaining_items = support::array(inventory.get("items"))
                        .into_iter()
                        .filter(|item| {
                            !support::item_is_completed(&contract, item, &completed_ids)
                        })
                        .collect();
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
                        &completed_ids,
                        dependency_iteration,
                        &mut accepted_this_wave,
                        &mut noop_reclassified_ids,
                        evidence,
                    )
                    .await?,
                );
                self.runtime_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .noop_reclassified_ids
                    .extend(noop_reclassified_ids.iter().cloned());
            }
            ready_implementation_items = self
                .discover_implementation_targets(
                    ready_implementation_items,
                    discovery,
                    dependency_iteration,
                    &noop_reclassified_ids,
                    evidence,
                )
                .await?;

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

            // The third point of the diamond: implementation-wave (FANOUT) ->
            // verification-wave (PARALLEL) -> adversarial-review (PARALLEL),
            // one reviewer per task, as soon as that task's own verification
            // accepted it. Running here rather than after every wave is what
            // makes a wave-1 defect cheap: it is reported before wave 2 builds
            // on it. The review is read-only and never gates wave completion —
            // its findings are carried to the review-remediation loop, which
            // is where work gets done about them.
            if !accepted_this_wave.is_empty() {
                self.run_per_task_adversarial_review(
                    &format!("wave-{dependency_iteration}"),
                    &accepted_this_wave,
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
                completed_ids.insert(id.clone());
                self.runtime_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .completed_ids
                    .insert(id);
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

    pub(super) async fn repair_dependency_deadlock(
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

    pub(super) async fn repair_wave_completion_evidence(
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

pub(super) fn item_has_write_ownership(item: &serde_json::Value) -> bool {
    support::present(item.get("target_files"))
        || support::present(item.get("artifact_requirements"))
}

pub(super) fn preserve_host_pinned_implementation(
    contract: &LifecycleContract<'_>,
    inventory: &serde_json::Value,
    noop_reclassified_ids: &std::collections::BTreeSet<String>,
) -> serde_json::Value {
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert(
        "items".to_string(),
        serde_json::Value::Array(preserve_host_pinned_items(
            contract,
            support::array(inventory.get("items")),
            noop_reclassified_ids,
        )),
    );
    contract.normalize_inventory(&serde_json::Value::Object(object))
}

pub(super) fn preserve_host_pinned_items(
    contract: &LifecycleContract<'_>,
    items: Vec<serde_json::Value>,
    noop_reclassified_ids: &std::collections::BTreeSet<String>,
) -> Vec<serde_json::Value> {
    items
        .into_iter()
        .map(|mut item| {
            if contract
                .canonical_ids_for(&item)
                .iter()
                .any(|id| noop_reclassified_ids.contains(id))
                && let Some(object) = item.as_object_mut()
            {
                object.insert(
                    "work_type".to_string(),
                    serde_json::Value::String("implementation".to_string()),
                );
            }
            item
        })
        .collect()
}

/// JS `generatedContractInventoryGraphIssues(remainingItems, completedIds)` —
/// deadlock diagnostics passed to the repair reducer.
pub(super) fn deadlock_graph_issues(
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
