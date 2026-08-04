use super::*;

pub(crate) fn collapse_outcome_clones(
    outcomes: &[serde_json::Value],
    limit: usize,
) -> (Vec<serde_json::Value>, usize) {
    let mut order = Vec::new();
    let mut grouped = std::collections::BTreeMap::<String, (serde_json::Value, usize)>::new();
    for outcome in outcomes {
        let stem = strip_check_suffixes(item_identity(outcome));
        let key = if stem.is_empty() {
            format!("anonymous-{}", order.len())
        } else {
            outcome_group_identity(stem, outcome)
        };
        if let Some((_, count)) = grouped.get_mut(&key) {
            *count += 1;
        } else {
            order.push(key.clone());
            grouped.insert(key, (slim_outcome(outcome), 1));
        }
    }
    let total = order.len();
    let collapsed = order
        .into_iter()
        .take(limit)
        .filter_map(|key| grouped.remove(&key))
        .map(|(mut outcome, count)| {
            if let Some(object) = outcome.as_object_mut() {
                object.insert("duplicate_count".to_string(), serde_json::json!(count));
            }
            outcome
        })
        .collect();
    (collapsed, total.saturating_sub(limit))
}

pub(crate) fn outcome_group_identity(stem: &str, outcome: &serde_json::Value) -> String {
    let result = outcome.get("result").unwrap_or(outcome);
    let mut gap_ids = support::array(result.get("residual_gaps"))
        .into_iter()
        .filter_map(|gap| {
            gap.get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    gap_ids.sort();
    gap_ids.dedup();
    if gap_ids.is_empty() {
        stem.to_string()
    } else {
        format!("{stem}::{}", gap_ids.join("|"))
    }
}

pub(crate) fn slim_outcome(outcome: &serde_json::Value) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for key in ["item_id", "id", "status", "failure_kind"] {
        if let Some(value) = outcome.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    let result = outcome.get("result").unwrap_or(outcome);
    out.insert("result".to_string(), slim_result_with_outcomes(result, 0));
    serde_json::Value::Object(out)
}

pub(crate) fn item_identity(value: &serde_json::Value) -> &str {
    value
        .get("item_id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

pub(crate) fn strip_check_suffixes(mut value: &str) -> &str {
    while let Some((stem, suffix)) = value.rsplit_once("-check-") {
        if !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            value = stem;
        } else {
            break;
        }
    }
    value
}

impl LifecycleDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_post_remediation_verification(
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
    ) -> crate::WorkflowResult<bool> {
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
        let post_items = lifecycle_policy::verify_options::prepare_verification_items(
            post_items,
            self.project_artifact_root.as_deref(),
            &evidence.implementation,
            &self.task_universe,
        );
        *verification = self
            .parallel(
                &format!("verification-wave-{wave_index}-post-remediation-{remediation_attempt}"),
                serde_json::json!(&post_items),
                lifecycle_policy::verify_options::verification_options(
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

    pub(crate) async fn post_remediation_plan(
        &self,
        ready_implementation_items: &[serde_json::Value],
        remediation_inventory: &serde_json::Value,
        remediation_wave: &serde_json::Value,
        wave_index: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
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

    pub(crate) async fn repair_post_remediation_plan(
        &self,
        contract: &LifecycleContract<'_>,
        remediation_inventory: &serde_json::Value,
        remediation_wave: serde_json::Value,
        raw_plan: serde_json::Value,
        wave_index: usize,
        remediation_attempt: &usize,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<Option<Vec<serde_json::Value>>> {
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
    pub async fn repair_post_remediation_plan_once(
        &self,
        remediation_inventory: &serde_json::Value,
        remediation_wave: &serde_json::Value,
        post_plan: serde_json::Value,
        wave_index: usize,
        remediation_attempt: &usize,
        shape_attempt: usize,
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
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
        let candidate = self.contract().normalize_inventory(&repair);
        // D74: keep the previous plan when the repair rewrote or dropped the
        // semantic identity of existing plan items; violations flow into the
        // next bounded attempt as unresolved issues.
        let preservation = semantic_preservation::check_items(
            &support::array(post_plan.get("items")),
            &support::array(candidate.get("items")),
        );
        if preservation.passed() {
            return Ok(candidate);
        }
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            &repair_id,
            "semantic_preservation_rejected",
            &semantic_preservation::violation_issues(&preservation.violations),
            &candidate,
        );
        self.record_preservation_rejection(&repair_id, &preservation.violations)
            .await?;
        let mut rejected = post_plan;
        semantic_preservation::append_preservation_issues(&mut rejected, &preservation.violations);
        Ok(rejected)
    }
}

pub(crate) fn producer_retry_items(
    contract: &LifecycleContract<'_>,
    producer_output: &serde_json::Value,
    producer: lifecycle_policy::verify_routing::RetryProducer,
    plan_items: &[serde_json::Value],
    source_outcomes: &[serde_json::Value],
) -> Option<Vec<serde_json::Value>> {
    let retry_items = lifecycle_policy::verify_routing::retry_items(producer_output);
    if lifecycle_policy::verify_routing::retry_consumption_route(producer, &retry_items)
        == lifecycle_policy::verify_routing::RetryConsumptionRoute::NotNeeded
    {
        return None;
    }
    let inventory = contract.normalize_inventory(&serde_json::json!({ "items": retry_items }));
    let inventory = lifecycle_policy::verify_invariants::enforce_retry_invariants(
        &inventory,
        &serde_json::json!({ "outcomes": source_outcomes }),
    );
    let allowed = allowed_verification_task_ids(plan_items);
    let constrained = support::constrain_inventory_tasks(contract, &inventory, &allowed);
    if !support::verification_inventory_ready(&constrained) {
        return None;
    }
    let items: Vec<serde_json::Value> = support::retry_verification_items(contract, &constrained)
        .into_iter()
        .filter(retry_item_requires_rerun)
        .collect();
    (!items.is_empty()).then_some(items)
}

pub(crate) fn retry_item_requires_rerun(item: &serde_json::Value) -> bool {
    let class = item
        .get("classification")
        .or_else(|| item.get("verification_failure_class"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let class = class.to_ascii_lowercase();
    !class.contains("sibling") && !class.contains("supersed")
}

pub(crate) fn allowed_verification_task_ids(plan_items: &[serde_json::Value]) -> Vec<String> {
    support::unique(
        plan_items
            .iter()
            .flat_map(|item| support::strings_of(item.get("canonical_task_ids")))
            .collect(),
    )
}

pub(crate) fn record_unresolved_verification_remediation(
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
