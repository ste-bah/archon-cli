// Verification-failure triage and remediation helpers for the native
// decomposed-PRD lifecycle.

impl LifecycleDriver {
    /// Triage → retry verification or write remediation → post-remediation
    /// re-verification. Returns whether the outer verification loop should
    /// continue (JS `continue`) or break.
    #[allow(clippy::too_many_arguments)]
    async fn run_verification_remediation(
        &self,
        ready_implementation_items: &[serde_json::Value],
        plan_items: &[serde_json::Value],
        actionable: &[serde_json::Value],
        wave_index: usize,
        dependency_iteration: usize,
        repair_attempt: usize,
        remediation_attempt: &mut usize,
        verification: &mut serde_json::Value,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<bool> {
        let contract = self.contract();
        let triage_id = format!("verification-failure-triage-{wave_index}-{repair_attempt}");
        let triage = self
            .reduce(
                &triage_id,
                serde_json::json!([
                    self.task_universe,
                    ready_implementation_items,
                    plan_items,
                    actionable,
                    evidence.implementation,
                    evidence.verification
                ]),
                "reducer",
                prompts::VERIFICATION_FAILURE_TRIAGE_TASK,
            )
            .await?;
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &triage_id,
            "verification_failure_triage",
            actionable,
            &triage,
        );
        let routes = workflow_live_v2_lifecycle_verify_routing::triage_routes(&triage);
        let mut retried = false;
        if let Some(retry_items) = triage_retry_items(&contract, &triage, plan_items, actionable) {
            let retry_items =
                workflow_live_v2_lifecycle_verify_options::prepare_verification_items(
                    retry_items,
                    self.project_artifact_root.as_deref(),
                );
            let retry_result = self
                .parallel(
                    &format!("verification-wave-{wave_index}-triage-retry-{repair_attempt}"),
                    serde_json::json!(&retry_items),
                    workflow_live_v2_lifecycle_verify_options::verification_options(
                        &retry_items,
                        prompts::RETRY_VERIFICATION_WAVE_TASK,
                        true,
                    ),
                )
                .await?;
            *verification = workflow_live_v2_lifecycle_verify_merge::merge_retry_outcomes(
                verification,
                retry_result,
                &retry_items,
            );
            evidence.verification.push(serde_json::json!({
                "kind": "verification-triage-retry",
                "implementationWaveIndex": wave_index,
                "dependencyIteration": dependency_iteration,
                "verificationRepairAttempt": repair_attempt,
                "verificationPlan": { "items": retry_items },
                "result": verification,
            }));
            retried = true;
        }
        if retried && routes.implementation_failures.is_empty() {
            return Ok(true);
        }
        let remediation_items = if routes.implementation_failures.is_empty() {
            actionable
        } else {
            &routes.implementation_failures
        };
        self.run_write_verification_remediation(
            ready_implementation_items,
            plan_items,
            remediation_items,
            wave_index,
            dependency_iteration,
            remediation_attempt,
            verification,
            evidence,
            &triage,
            &triage_id,
        )
        .await
    }

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
        triage_id: &str,
    ) -> archon_workflow::WorkflowResult<bool> {
        let contract = self.contract();
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
        if !remediation::remediation_inventory_ready(&remediation_inventory) {
            if let Some(supersede) =
                workflow_live_v2_lifecycle_verify_supersede::try_supersede_verification(
                    &contract,
                    verification,
                    triage,
                    triage_id,
                )
            {
                *verification = supersede.verification;
                evidence.verification.push(supersede.record);
                return Ok(true);
            }
            return Ok(false);
        }
        let remediation_wave = self
            .run_verification_remediation_wave(
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

    async fn run_verification_remediation_wave(
        &self,
        remediation_inventory: &serde_json::Value,
        wave_index: usize,
        dependency_iteration: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        let source_items =
            workflow_live_v2_lifecycle_verify_merge::verification_remediation_source_items(
                remediation_inventory,
            );
        let remediation_wave = self
            .write_fanout(
                &format!("remediation-wave-{wave_index}-verification-{remediation_attempt}"),
                serde_json::json!(source_items),
                prompts::VERIFICATION_REMEDIATION_WAVE_TASK,
            )
            .await?;
        evidence.implementation.push(serde_json::json!({
            "kind": "verification-remediation",
            "implementationWaveIndex": wave_index,
            "dependencyIteration": dependency_iteration,
            "verificationRemediationAttempt": remediation_attempt,
            "verificationRemediationInventory": remediation_inventory,
            "result": remediation_wave,
        }));
        record_unresolved_verification_remediation(
            remediation_attempt,
            wave_index,
            evidence,
            &remediation_wave,
        );
        Ok(remediation_wave)
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
    !class.contains("resolved")
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
