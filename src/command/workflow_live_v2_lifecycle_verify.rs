// Focused-verification lifecycle for the Rust decomposed-PRD lifecycle:
// planning, verification waves, failure triage, write remediation, and retry
// plans — ported faithfully from the spliced verification lifecycle
// (workflow_live_generated_scaffold_verification.rs::VERIFICATION_LIFECYCLE_JS).

impl LifecycleDriver {
    pub(in super::super) async fn run_verification_lifecycle(
        &self,
        ready_implementation_items: &[serde_json::Value],
        implementation_candidate_ids_unique: &[String],
        wave_index: usize,
        dependency_iteration: usize,
        accepted_this_wave: &mut std::collections::BTreeSet<String>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<()> {
        let contract = self.contract();

        let raw_plan = self
            .reduce(
                &format!("verification-plan-{wave_index}"),
                serde_json::json!([
                    self.task_universe,
                    ready_implementation_items,
                    implementation_candidate_ids_unique,
                    evidence.implementation
                ]),
                "reducer",
                prompts::VERIFICATION_PLAN_TASK,
            )
            .await?;
        let mut verification_plan = contract.normalize_inventory(&raw_plan);
        let mut plan_repair_attempt = 1usize;
        while !support::verification_inventory_ready(&verification_plan)
            && plan_repair_attempt <= self.max_repair_iterations
        {
            let call_id = format!("verification-plan-repair-{wave_index}-{plan_repair_attempt}");
            let repair = self
                .reduce(
                    &call_id,
                    serde_json::json!([
                        self.task_universe,
                        ready_implementation_items,
                        implementation_candidate_ids_unique,
                        evidence.implementation,
                        verification_plan
                    ]),
                    "reducer",
                    prompts::VERIFICATION_PLAN_REPAIR_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &call_id,
                "verification_plan_repair",
                &implementation_candidate_ids_unique
                    .iter()
                    .map(|id| serde_json::json!({ "canonical_task_ids": [id] }))
                    .collect::<Vec<_>>(),
                &repair,
            );
            verification_plan = contract.normalize_inventory(&repair);
            plan_repair_attempt += 1;
        }
        let plan_items = if support::verification_inventory_ready(&verification_plan) {
            support::verification_items(&contract, &verification_plan)
        } else {
            Vec::new()
        };
        if plan_items.is_empty() {
            return self
                .final_report(
                    &format!("blocked-empty-verification-{wave_index}"),
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "readyImplementationItems": ready_implementation_items,
                        "implementationCandidateIdsUnique": implementation_candidate_ids_unique,
                        "verificationPlan": verification_plan,
                        "implementationEvidence": evidence.implementation,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_EMPTY_VERIFICATION_TASK,
                )
                .await;
        }
        let mut plan_items = workflow_live_v2_lifecycle_verify_options::prepare_verification_items(
            plan_items,
            self.project_artifact_root.as_deref(),
            &evidence.implementation,
        );

        let mut verification = self
            .parallel(
                &format!("verification-wave-{wave_index}"),
                serde_json::json!(&plan_items),
                workflow_live_v2_lifecycle_verify_options::verification_options(
                    &plan_items,
                    prompts::VERIFICATION_WAVE_TASK,
                    true,
                ),
            )
            .await?;
        evidence.verification.push(serde_json::json!({
            "kind": "verification",
            "implementationWaveIndex": wave_index,
            "dependencyIteration": dependency_iteration,
            "verificationPlan": { "items": plan_items },
            "result": verification,
        }));

        let mut repair_attempt = 1usize;
        let mut remediation_attempt = 1usize;
        while !support::outcome_accepted_or_noop(&verification)
            && repair_attempt <= self.max_repair_iterations
        {
            let outcomes = support::outcomes_of(&verification);
            let actionable: Vec<serde_json::Value> = outcomes
                .iter()
                .filter(|outcome| {
                    let data = outcome
                        .get("result")
                        .and_then(|result| result.get("data"))
                        .or_else(|| outcome.get("data"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    data.get("verification_remediation_required")
                        == Some(&serde_json::Value::Bool(true))
                        || data.get("verification_failure_class").and_then(|v| v.as_str())
                            == Some("actionable_implementation_failure")
                        || data
                            .get("verification_failure_next_action")
                            .and_then(|v| v.as_str())
                            == Some("write_remediation")
                })
                .cloned()
                .collect();

            if !actionable.is_empty() {
                let continue_loop = self
                    .run_verification_remediation(
                        ready_implementation_items,
                        &plan_items,
                        wave_index,
                        dependency_iteration,
                        repair_attempt,
                        &mut remediation_attempt,
                        &mut verification,
                        evidence,
                    )
                    .await?;
                if !continue_loop {
                    break;
                }
                repair_attempt += 1;
                continue;
            }

            let repeated = workflow_live_v2_lifecycle_verify_routing::repeated_gap_write_remediation_outcomes(
                &evidence.verification,
                &verification,
            );
            if !repeated.is_empty() {
                let call_id = format!(
                    "verification-repeated-gap-escalation-{wave_index}-{repair_attempt}"
                );
                let source_ids = repeated
                    .iter()
                    .filter_map(|outcome| outcome.get("item_id").and_then(serde_json::Value::as_str))
                    .collect::<Vec<_>>();
                let route = serde_json::json!({
                    "status": "accepted",
                    "data": {
                        "route": "write_remediation",
                        "route_reason": "same residual gap reproduced across two retry generations",
                        "source_outcome_ids": source_ids,
                    }
                });
                support::record_repair_attempt(
                    &mut evidence.repair_attempts,
                    &call_id,
                    "verification_repeated_gap_escalation",
                    &repeated,
                    &route,
                );
                let continue_loop = self
                    .run_write_verification_remediation(
                        ready_implementation_items,
                        &plan_items,
                        &repeated,
                        wave_index,
                        dependency_iteration,
                        &mut remediation_attempt,
                        &mut verification,
                        evidence,
                        &route,
                    )
                    .await?;
                if !continue_loop {
                    break;
                }
                repair_attempt += 1;
                continue;
            }

            let call_id = format!("verification-repair-plan-{wave_index}-{repair_attempt}");
            let repair_plan = self
                .reduce(
                    &call_id,
                    serde_json::json!([
                        self.task_universe,
                        plan_items,
                        verification,
                        evidence.implementation
                    ]),
                    "reducer",
                    prompts::VERIFICATION_REPAIR_PLAN_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &call_id,
                "verification_repair",
                &support::outcomes_of(&verification),
                &repair_plan,
            );
            let routed = workflow_live_v2_lifecycle_verify_routing::write_remediation_outcomes(
                &repair_plan,
                &verification,
            );
            if !routed.is_empty() {
                let continue_loop = self
                    .run_write_verification_remediation(
                        ready_implementation_items,
                        &plan_items,
                        &routed,
                        wave_index,
                        dependency_iteration,
                        &mut remediation_attempt,
                        &mut verification,
                        evidence,
                        &repair_plan,
                    )
                    .await?;
                if !continue_loop {
                    break;
                }
                repair_attempt += 1;
                continue;
            }
            let repair_source = workflow_live_v2_lifecycle_verify_routing::predicate_rewrite_inventory(
                &repair_plan,
                &verification,
            )
            .unwrap_or_else(|| repair_plan.clone());
            let mut repair_inventory = contract.normalize_inventory(&repair_source);
            repair_inventory = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
                &repair_inventory,
                &verification,
            );
            let allowed_task_ids: Vec<String> = support::unique(
                plan_items
                    .iter()
                    .flat_map(|item| support::strings_of(item.get("canonical_task_ids")))
                    .collect(),
            );
            repair_inventory =
                support::constrain_inventory_tasks(&contract, &repair_inventory, &allowed_task_ids);
            let mut shape_attempt = 1usize;
            while !support::verification_inventory_ready(&repair_inventory)
                && !support::array(repair_inventory.get("unresolved_issues")).is_empty()
                && shape_attempt <= self.max_repair_iterations
            {
                let shape_call_id = format!(
                    "verification-repair-shape-repair-{wave_index}-{repair_attempt}-{shape_attempt}"
                );
                let issues = support::array(repair_inventory.get("unresolved_issues"));
                let shape_repair = self
                    .reduce(
                        &shape_call_id,
                        serde_json::json!([
                            self.task_universe,
                            repair_inventory,
                            issues,
                            verification,
                            evidence.implementation
                        ]),
                        "reducer",
                        prompts::VERIFICATION_REPAIR_SHAPE_REPAIR_TASK,
                    )
                    .await?;
                support::record_repair_attempt(
                    &mut evidence.repair_attempts,
                    &shape_call_id,
                    "verification_repair_shape_repair",
                    &issues,
                    &shape_repair,
                );
                repair_inventory = contract.normalize_inventory(&shape_repair);
                repair_inventory = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
                    &repair_inventory,
                    &verification,
                );
                repair_inventory = support::constrain_inventory_tasks(
                    &contract,
                    &repair_inventory,
                    &allowed_task_ids,
                );
                shape_attempt += 1;
            }
            if !support::verification_inventory_ready(&repair_inventory)
                || support::array(repair_inventory.get("items")).is_empty()
            {
                break;
            }
            repair_inventory = scope_repair_inventory_to_failed_outcomes(
                &contract,
                &repair_inventory,
                &verification,
            );
            if support::array(repair_inventory.get("items")).is_empty() {
                break;
            }
            plan_items = support::verification_items(&contract, &repair_inventory);
            plan_items = workflow_live_v2_lifecycle_verify_options::prepare_verification_items(
                plan_items,
                self.project_artifact_root.as_deref(),
                &evidence.implementation,
            );
            verification = self
                .parallel(
                    &format!("verification-wave-{wave_index}-{repair_attempt}"),
                    serde_json::json!(&plan_items),
                    workflow_live_v2_lifecycle_verify_options::verification_options(
                        &plan_items,
                        prompts::RETRY_VERIFICATION_WAVE_TASK,
                        true,
                    ),
                )
                .await?;
            evidence.verification.push(serde_json::json!({
                "kind": "verification-retry",
                "implementationWaveIndex": wave_index,
                "dependencyIteration": dependency_iteration,
                "verificationRepairAttempt": repair_attempt,
                "verificationPlan": { "items": plan_items },
                "result": verification,
            }));
            repair_attempt += 1;
        }

        if !support::outcome_accepted_or_noop(&verification) {
            return self
                .final_report(
                    &format!("blocked-verification-failed-{wave_index}"),
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "readyImplementationItems": ready_implementation_items,
                        "implementationCandidateIdsUnique": implementation_candidate_ids_unique,
                        "implementationEvidence": evidence.implementation,
                        "verificationEvidence": evidence.verification,
                        "verification": verification,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_VERIFICATION_FAILED_TASK,
                )
                .await;
        }
        for id in support::accepted_or_noop_canonical_task_ids_from(
            &contract,
            &support::outcomes_of(&verification),
        ) {
            if implementation_candidate_ids_unique.contains(&id) {
                accepted_this_wave.insert(id);
            }
        }
        Ok(())
    }

}

fn scope_repair_inventory_to_failed_outcomes(
    contract: &LifecycleContract<'_>,
    inventory: &serde_json::Value,
    verification: &serde_json::Value,
) -> serde_json::Value {
    let outcomes = support::outcomes_of(verification);
    let failed_ids = outcome_ids(&support::non_accepted_outcomes(&outcomes));
    let accepted: Vec<serde_json::Value> = outcomes
        .iter()
        .filter(|outcome| support::outcome_accepted_or_noop(outcome))
        .cloned()
        .collect();
    let accepted_ids = outcome_ids(&accepted);
    if failed_ids.is_empty() || accepted_ids.is_empty() {
        return inventory.clone();
    }
    let items = support::array(inventory.get("items"))
        .into_iter()
        .map(|item| contract.normalize_item(&item))
        .filter(|item| item_matches_ids(item, &failed_ids) || !item_matches_ids(item, &accepted_ids))
        .collect();
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), serde_json::Value::Array(items));
    serde_json::Value::Object(object)
}

fn outcome_ids(outcomes: &[serde_json::Value]) -> std::collections::BTreeSet<String> {
    outcomes.iter().flat_map(item_match_ids).collect()
}

fn item_matches_ids(
    item: &serde_json::Value,
    ids: &std::collections::BTreeSet<String>,
) -> bool {
    item_match_ids(item).into_iter().any(|id| ids.contains(&id))
}

fn item_match_ids(item: &serde_json::Value) -> Vec<String> {
    workflow_live_v2_lifecycle_verify_invariants::verification_item_ids(item)
}
