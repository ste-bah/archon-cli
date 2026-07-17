// Implementation-wave execution for the Rust decomposed-PRD lifecycle:
// write fanout, remediation inventory + waves, follow-up remediation, and the
// ownership-expansion follow-up (body_a.js implementation block plus the
// ownership splice), ported faithfully.

impl LifecycleDriver {
    pub(in super::super) async fn run_implementation_wave(
        &self,
        ready_implementation_items: &[serde_json::Value],
        wave_index: usize,
        dependency_iteration: usize,
        implementation_candidate_ids: &mut Vec<String>,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let contract = self.contract();
        let wave = self
            .write_fanout(
                &format!("implementation-wave-{wave_index}"),
                serde_json::json!(ready_implementation_items),
                prompts::IMPLEMENTATION_WAVE_TASK,
            )
            .await?;
        evidence.implementation.push(serde_json::json!({
            "kind": "implementation",
            "implementationWaveIndex": wave_index,
            "dependencyIteration": dependency_iteration,
            "readyImplementationItems": ready_implementation_items,
            "result": wave,
        }));
        implementation_candidate_ids.extend(support::matching_accepted_ids(
            &contract,
            ready_implementation_items,
            &support::outcomes_of(&wave),
        ));

        let failed_outcomes = support::non_accepted_outcomes(&support::outcomes_of(&wave));
        if failed_outcomes.is_empty() {
            return Ok(wave);
        }

        let raw_remediation = self
            .reduce(
                &format!("remediation-inventory-{wave_index}"),
                serde_json::json!([
                    self.task_universe,
                    ready_implementation_items,
                    wave,
                    failed_outcomes,
                    evidence.implementation
                ]),
                "reducer",
                prompts::REMEDIATION_INVENTORY_TASK,
            )
            .await?;
        let source_call_id = format!("implementation-wave-{wave_index}");
        let mut remediation_inventory = remediation::normalize_remediation_inventory_for_sources(
            &contract,
            &raw_remediation,
            ready_implementation_items,
            &[],
            &source_call_id,
        );
        let mut repair_attempt = 1usize;
        while !remediation::remediation_inventory_ready(&remediation_inventory)
            && repair_attempt <= self.max_repair_iterations
        {
            let call_id = format!("remediation-empty-inventory-repair-{wave_index}-{repair_attempt}");
            let repair = self
                .reduce(
                    &call_id,
                    serde_json::json!([
                        self.task_universe,
                        ready_implementation_items,
                        failed_outcomes,
                        remediation_inventory
                    ]),
                    "reducer",
                    prompts::REMEDIATION_EMPTY_INVENTORY_REPAIR_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &call_id,
                "remediation_inventory_repair",
                &failed_outcomes,
                &repair,
            );
            let previous_items = support::array(remediation_inventory.get("items"));
            remediation_inventory = remediation::normalize_remediation_inventory_for_sources(
                &contract,
                &repair,
                ready_implementation_items,
                &previous_items,
                &source_call_id,
            );
            repair_attempt += 1;
        }
        if !remediation::remediation_inventory_ready(&remediation_inventory) {
            return self
                .final_report(
                    &format!("blocked-malformed-remediation-{wave_index}"),
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "readyImplementationItems": ready_implementation_items,
                        "wave": wave,
                        "failedImplementationOutcomes": failed_outcomes,
                        "remediationInventory": remediation_inventory,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_MALFORMED_REMEDIATION_TASK,
                )
                .await
                .map(|()| wave.clone());
        }

        let mut remediation_wave = self
            .write_fanout(
                &format!("remediation-wave-{wave_index}"),
                serde_json::json!(support::array(remediation_inventory.get("items"))),
                prompts::REMEDIATION_WAVE_TASK,
            )
            .await?;
        evidence.implementation.push(serde_json::json!({
            "kind": "remediation",
            "implementationWaveIndex": wave_index,
            "dependencyIteration": dependency_iteration,
            "remediationInventory": remediation_inventory,
            "result": remediation_wave,
        }));
        implementation_candidate_ids.extend(support::matching_accepted_ids(
            &contract,
            &support::array(remediation_inventory.get("items")),
            &support::outcomes_of(&remediation_wave),
        ));
        let mut unresolved =
            support::non_accepted_outcomes(&support::outcomes_of(&remediation_wave));
        let remediation_task_ids = remediation::remediation_task_id_set(
            &contract,
            &support::array(remediation_inventory.get("items")),
        );
        let mut unscheduled_followup: Option<serde_json::Value> = None;
        let mut remediation_attempt = 1usize;
        while !unresolved.is_empty() && remediation_attempt <= self.max_repair_iterations {
            let call_id = format!("remediation-outcome-repair-{wave_index}-{remediation_attempt}");
            let followup_raw = self
                .reduce(
                    &call_id,
                    serde_json::json!([
                        self.task_universe,
                        ready_implementation_items,
                        support::array(remediation_inventory.get("items")),
                        remediation_wave,
                        unresolved
                    ]),
                    "reducer",
                    prompts::REMEDIATION_OUTCOME_REPAIR_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &call_id,
                "remediation_inventory_repair",
                &unresolved,
                &followup_raw,
            );
            let followup_inventory = self
                .enforce_outcome_repair_accounting(
                    &call_id,
                    followup_raw,
                    &unresolved,
                    &support::array(remediation_inventory.get("items")),
                    ready_implementation_items,
                    &format!("remediation-wave-{wave_index}"),
                    &remediation_task_ids,
                    evidence,
                )
                .await?;
            if !remediation::remediation_inventory_ready(&followup_inventory) {
                unscheduled_followup = Some(followup_inventory);
                break;
            }
            let followup_wave = self
                .write_fanout(
                    &format!("remediation-wave-{wave_index}-{remediation_attempt}"),
                    serde_json::json!(support::array(followup_inventory.get("items"))),
                    prompts::FOLLOWUP_REMEDIATION_WAVE_TASK,
                )
                .await?;
            evidence.implementation.push(serde_json::json!({
                "kind": "remediation-retry",
                "implementationWaveIndex": wave_index,
                "dependencyIteration": dependency_iteration,
                "remediationAttempt": remediation_attempt,
                "remediationInventory": followup_inventory,
                "result": followup_wave,
            }));
            implementation_candidate_ids.extend(support::matching_accepted_ids(
                &contract,
                &support::array(followup_inventory.get("items")),
                &support::outcomes_of(&followup_wave),
            ));
            remediation_inventory = followup_inventory;
            remediation_wave = followup_wave;
            unresolved = support::non_accepted_outcomes(&support::outcomes_of(&remediation_wave));
            remediation_attempt += 1;
        }

        // Ownership-expansion splice: unresolved outcomes that carry explicit
        // ownership-expansion evidence get exactly one follow-up wave.
        if !unresolved.is_empty() {
            let expansion_outcomes: Vec<serde_json::Value> = unresolved
                .iter()
                .filter(|outcome| {
                    let data = outcome
                        .get("result")
                        .and_then(|result| result.get("data"))
                        .or_else(|| outcome.get("data"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    data.get("ownership_expansion_required") == Some(&serde_json::Value::Bool(true))
                        || !support::array(data.get("proposed_ownership_expansions")).is_empty()
                })
                .cloned()
                .collect();
            if !expansion_outcomes.is_empty() {
                let call_id =
                    format!("ownership-expansion-inventory-{wave_index}-{remediation_attempt}");
                let expansion_raw = self
                    .reduce(
                        &call_id,
                        serde_json::json!([
                            self.task_universe,
                            ready_implementation_items,
                            support::array(remediation_inventory.get("items")),
                            remediation_wave,
                            expansion_outcomes,
                            evidence.implementation
                        ]),
                        "reducer",
                        prompts::OWNERSHIP_EXPANSION_INVENTORY_TASK,
                    )
                    .await?;
                support::record_repair_attempt(
                    &mut evidence.repair_attempts,
                    &call_id,
                    "ownership_expansion_inventory",
                    &expansion_outcomes,
                    &expansion_raw,
                );
                let expansion_inventory =
                    remediation::normalize_remediation_inventory(&contract, &expansion_raw);
                let expansion_ready =
                    remediation::remediation_inventory_ready(&expansion_inventory);
                if !expansion_ready {
                    unscheduled_followup.get_or_insert(expansion_inventory.clone());
                }
                if expansion_ready {
                    let expansion_wave = self
                        .write_fanout(
                            &format!(
                                "remediation-wave-{wave_index}-ownership-{remediation_attempt}"
                            ),
                            serde_json::json!(support::array(expansion_inventory.get("items"))),
                            prompts::OWNERSHIP_EXPANSION_WAVE_TASK,
                        )
                        .await?;
                    evidence.implementation.push(serde_json::json!({
                        "kind": "ownership-expansion-remediation",
                        "implementationWaveIndex": wave_index,
                        "dependencyIteration": dependency_iteration,
                        "remediationAttempt": remediation_attempt,
                        "remediationInventory": expansion_inventory,
                        "result": expansion_wave,
                    }));
                    implementation_candidate_ids.extend(support::matching_accepted_ids(
                        &contract,
                        &support::array(expansion_inventory.get("items")),
                        &support::outcomes_of(&expansion_wave),
                    ));
                    remediation_inventory = expansion_inventory;
                    remediation_wave = expansion_wave;
                    unresolved =
                        support::non_accepted_outcomes(&support::outcomes_of(&remediation_wave));
                }
            }
        }

        if !unresolved.is_empty() {
            return self
                .final_report(
                    &format!("blocked-remediation-unresolved-{wave_index}"),
                    None,
                    "needs_review",
                    serde_json::json!({
                        "taskUniverse": self.task_universe,
                        "readyImplementationItems": ready_implementation_items,
                        "wave": wave,
                        "remediationInventory": remediation_inventory,
                        "remediationWave": remediation_wave,
                        "unresolvedAfterRemediation": unresolved,
                        "unscheduledFollowupInventory": unscheduled_followup,
                        "implementationEvidence": evidence.implementation,
                        "repair_attempts": evidence.repair_attempts,
                    }),
                    prompts::BLOCKED_REMEDIATION_UNRESOLVED_TASK,
                )
                .await
                .map(|()| wave.clone());
        }
        Ok(wave)
    }
}
