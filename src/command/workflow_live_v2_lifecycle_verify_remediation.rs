// Verification-failure triage and remediation helpers for the native
// decomposed-PRD lifecycle.

impl LifecycleDriver {
    #[allow(clippy::too_many_arguments)]
    async fn run_write_verification_remediation(
        &self,
        ready_implementation_items: &[serde_json::Value],
        plan_items: &[serde_json::Value],
        actionable: &[serde_json::Value],
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &mut usize,
        verification: &mut serde_json::Value,
        evidence: &mut LifecycleEvidence,
        triage: &serde_json::Value,
    ) -> archon_workflow::WorkflowResult<bool> {
        let remediation_inventory = self
            .verification_remediation_inventory(
                ready_implementation_items,
                plan_items,
                actionable,
                wave_index,
                remediation_attempt,
                evidence,
                triage,
            )
            .await?;
        let mut routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(triage);
        if routes.implementation_failures.is_empty() {
            routes.implementation_failures = actionable.to_vec();
        }
        let route_plan = workflow_live_v2_lifecycle_verify_routing::triage_route_plan(&routes);
        match workflow_live_v2_lifecycle_verify_routing::remediation_inventory_route(
            &route_plan,
            remediation::remediation_inventory_ready(&remediation_inventory),
        ) {
            workflow_live_v2_lifecycle_verify_routing::RemediationInventoryRoute::RunWriteRemediation => {}
            workflow_live_v2_lifecycle_verify_routing::RemediationInventoryRoute::RegenerateInventory => {
                return Ok(true);
            }
            workflow_live_v2_lifecycle_verify_routing::RemediationInventoryRoute::NotNeeded
            | workflow_live_v2_lifecycle_verify_routing::RemediationInventoryRoute::Block => {
                return Ok(false);
            }
        }
        let contract = self.contract();
        let remediation_wave = self
            .run_verification_remediation_wave(
                ready_implementation_items,
                &remediation_inventory,
                wave_index,
                dependency_iteration,
                remediation_attempt,
                evidence,
            )
            .await?;
        if !support::non_accepted_outcomes(&support::outcomes_of(&remediation_wave)).is_empty() {
            return Ok(false);
        }
        self.run_post_remediation_verification(
            ready_implementation_items,
            &remediation_inventory,
            remediation_wave,
            wave_index,
            dependency_iteration,
            remediation_attempt,
            verification,
            evidence,
            &contract,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn verification_remediation_inventory(
        &self,
        ready_implementation_items: &[serde_json::Value],
        plan_items: &[serde_json::Value],
        actionable: &[serde_json::Value],
        wave_index: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
        triage: &serde_json::Value,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let inventory_id =
            format!("verification-remediation-inventory-{wave_index}-{remediation_attempt}");
        let raw_inventory = self
            .reduce(
                &inventory_id,
                serde_json::json!([
                    self.task_universe,
                    ready_implementation_items,
                    plan_items,
                    triage,
                    actionable,
                    evidence.implementation,
                    evidence.verification
                ]),
                "reducer",
                prompts::VERIFICATION_REMEDIATION_INVENTORY_TASK,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &inventory_id,
            "verification_remediation_inventory",
            actionable,
            &raw_inventory,
        );
        Ok(remediation::normalize_remediation_inventory_for_sources(
            &self.contract(),
            &raw_inventory,
            ready_implementation_items,
            &[],
            &format!("verification-wave-{wave_index}"),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_post_remediation_verification(
        &self,
        ready_implementation_items: &[serde_json::Value],
        remediation_inventory: &serde_json::Value,
        remediation_wave: serde_json::Value,
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &mut usize,
        verification: &mut serde_json::Value,
        evidence: &mut LifecycleEvidence,
        contract: &LifecycleContract<'_>,
    ) -> archon_workflow::WorkflowResult<bool> {
        let raw_plan = self
            .post_remediation_plan(
                ready_implementation_items,
                remediation_inventory,
                &remediation_wave,
                wave_index,
                remediation_attempt,
                evidence,
            )
            .await?;
        let Some(post_items) = self
            .repair_post_remediation_plan(
                contract,
                remediation_inventory,
                remediation_wave,
                raw_plan,
                wave_index,
                remediation_attempt,
                evidence,
            )
            .await?
        else {
            return Ok(false);
        };
        let post_items = workflow_live_v2_lifecycle_verify_options::prepare_verification_items(
            post_items,
            self.project_artifact_root.as_deref(),
            &evidence.implementation,
        );
        *verification = self
            .parallel(
                &format!("verification-wave-{wave_index}-post-remediation-{remediation_attempt}"),
                serde_json::json!(&post_items),
                workflow_live_v2_lifecycle_verify_options::verification_options(
                    &post_items,
                    prompts::POST_REMEDIATION_VERIFICATION_WAVE_TASK,
                    true,
                ),
            )
            .await?;
        evidence.verification.push(serde_json::json!({
            "kind": "post-remediation-verification",
            "implementationWaveIndex": wave_index,
            "dependencyIteration": dependency_iteration,
            "verificationRemediationAttempt": *remediation_attempt,
            "verificationPlan": { "items": post_items },
            "result": verification,
        }));
        *remediation_attempt += 1;
        Ok(true)
    }

    async fn post_remediation_plan(
        &self,
        ready_implementation_items: &[serde_json::Value],
        remediation_inventory: &serde_json::Value,
        remediation_wave: &serde_json::Value,
        wave_index: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let plan_id =
            format!("post-remediation-verification-plan-{wave_index}-{remediation_attempt}");
        let raw_plan = self
            .reduce(
                &plan_id,
                serde_json::json!([
                    self.task_universe,
                    ready_implementation_items,
                    support::array(remediation_inventory.get("items")),
                    remediation_wave,
                    evidence.implementation,
                    evidence.verification
                ]),
                "reducer",
                prompts::POST_REMEDIATION_VERIFICATION_PLAN_TASK,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &plan_id,
            "post_remediation_verification_plan",
            &support::array(remediation_inventory.get("items")),
            &raw_plan,
        );
        Ok(raw_plan)
    }

    async fn repair_post_remediation_plan(
        &self,
        contract: &LifecycleContract<'_>,
        remediation_inventory: &serde_json::Value,
        remediation_wave: serde_json::Value,
        raw_plan: serde_json::Value,
        wave_index: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<Option<Vec<serde_json::Value>>> {
        let mut post_plan = contract.normalize_inventory(&raw_plan);
        let mut shape_attempt = 1usize;
        while !support::verification_inventory_ready(&post_plan)
            && shape_attempt <= self.max_repair_iterations
        {
            post_plan = self
                .repair_post_remediation_plan_once(
                    remediation_inventory,
                    &remediation_wave,
                    post_plan,
                    wave_index,
                    remediation_attempt,
                    shape_attempt,
                    evidence,
                )
                .await?;
            shape_attempt += 1;
        }
        if !support::verification_inventory_ready(&post_plan) {
            return Ok(None);
        }
        let items = support::verification_items(contract, &post_plan);
        Ok((!items.is_empty()).then_some(items))
    }

    #[allow(clippy::too_many_arguments)]
    async fn repair_post_remediation_plan_once(
        &self,
        remediation_inventory: &serde_json::Value,
        remediation_wave: &serde_json::Value,
        post_plan: serde_json::Value,
        wave_index: usize,
        remediation_attempt: &usize,
        shape_attempt: usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let repair_id = format!(
            "post-remediation-verification-plan-repair-{wave_index}-{remediation_attempt}-{shape_attempt}"
        );
        let issues = support::array(post_plan.get("unresolved_issues"));
        let repair = self
            .reduce(
                &repair_id,
                serde_json::json!([
                    self.task_universe,
                    support::array(remediation_inventory.get("items")),
                    remediation_wave,
                    post_plan
                ]),
                "reducer",
                prompts::POST_REMEDIATION_VERIFICATION_PLAN_REPAIR_TASK,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &repair_id,
            "post_remediation_verification_plan_repair",
            &issues,
            &repair,
        );
        Ok(self.contract().normalize_inventory(&repair))
    }
}

fn triage_retry_items(
    contract: &LifecycleContract<'_>,
    triage: &serde_json::Value,
    plan_items: &[serde_json::Value],
    source_outcomes: &[serde_json::Value],
) -> Option<Vec<serde_json::Value>> {
    let retry_items = workflow_live_v2_lifecycle_verify_routing::triage_routes(triage).retry_items;
    let inventory = contract.normalize_inventory(&serde_json::json!({ "items": retry_items }));
    let inventory = workflow_live_v2_lifecycle_verify_invariants::enforce_retry_invariants(
        &inventory,
        &serde_json::json!({ "outcomes": source_outcomes }),
    );
    let allowed = allowed_verification_task_ids(plan_items);
    let constrained = support::constrain_inventory_tasks(contract, &inventory, &allowed);
    if !support::verification_inventory_ready(&constrained) {
        return None;
    }
    let items: Vec<serde_json::Value> = support::verification_items(contract, &constrained)
        .into_iter()
        .filter(retry_item_requires_rerun)
        .collect();
    (!items.is_empty()).then_some(items)
}

fn retry_item_requires_rerun(item: &serde_json::Value) -> bool {
    let class = item
        .get("classification")
        .or_else(|| item.get("verification_failure_class"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let class = class.to_ascii_lowercase();
    !class.contains("sibling") && !class.contains("supersed")
}

fn allowed_verification_task_ids(plan_items: &[serde_json::Value]) -> Vec<String> {
    support::unique(
        plan_items
            .iter()
            .flat_map(|item| support::strings_of(item.get("canonical_task_ids")))
            .collect(),
    )
}

fn record_unresolved_verification_remediation(
    remediation_attempt: &usize,
    wave_index: usize,
    evidence: &mut LifecycleEvidence,
    remediation_wave: &serde_json::Value,
) {
    let unresolved = support::non_accepted_outcomes(&support::outcomes_of(remediation_wave));
    if unresolved.is_empty() {
        return;
    }
    support::record_repair_attempt(
        &mut evidence.repair_attempts,
        &format!("remediation-wave-{wave_index}-verification-{remediation_attempt}"),
        "verification_remediation_unresolved",
        &unresolved,
        remediation_wave,
    );
}
