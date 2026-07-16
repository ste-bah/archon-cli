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
        let max_transport_attempts = self.max_repair_iterations.clamp(1, 2);
        let mut raw_inventory = serde_json::Value::Null;
        for transport_attempt in 1..=max_transport_attempts {
            let call_id = if transport_attempt == 1 {
                inventory_id.clone()
            } else {
                format!("{inventory_id}-regenerate-{transport_attempt}")
            };
            let source = verification_remediation_inventory_source(
                &self.task_universe,
                ready_implementation_items,
                plan_items,
                triage,
                actionable,
                &evidence.implementation,
                &evidence.verification,
                transport_attempt > 1,
            );
            raw_inventory = match self
                .reduce(
                    &call_id,
                    source,
                    "reducer",
                    prompts::VERIFICATION_REMEDIATION_INVENTORY_TASK,
                )
                .await
            {
                Ok(result) => result,
                Err(error) if is_transport_failure_text(&error.to_string()) => {
                    transport_failure_result(&call_id, 1, &error.to_string())
                }
                Err(error) => return Err(error),
            };
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &call_id,
                "verification_remediation_inventory",
                actionable,
                &raw_inventory,
            );
            match inventory_transport_route(
                &raw_inventory,
                transport_attempt,
                max_transport_attempts,
            ) {
                InventoryTransportRoute::Retry => continue,
                InventoryTransportRoute::UseResult | InventoryTransportRoute::Exhausted(_) => break,
            }
        }
        Ok(remediation::normalize_remediation_inventory_for_sources(
            &self.contract(),
            &raw_inventory,
            ready_implementation_items,
            &[],
            &format!("verification-wave-{wave_index}"),
        ))
    }
}

fn transport_failure_result(
    call_id: &str,
    attempts: usize,
    error: &str,
) -> serde_json::Value {
    serde_json::json!({
        "status": "failed",
        "summary": format!(
            "reducer transport exhausted after {attempts} attempt(s) for '{call_id}': {error}"
        ),
        "data": {
            "error": error,
            "failure_class": "transport_infrastructure",
            "transport_exhausted": true,
            "transport_attempts": attempts,
            "terminal_blockers": [{
                "id": format!("transport-exhausted-{call_id}"),
                "classification": "transport_infrastructure_exhausted",
                "description": error,
                "call_id": call_id,
                "attempts": attempts,
            }],
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
enum InventoryTransportRoute {
    UseResult,
    Retry,
    Exhausted(String),
}

fn inventory_transport_route(
    result: &serde_json::Value,
    attempt: usize,
    max_attempts: usize,
) -> InventoryTransportRoute {
    let Some(error) = transport_failure_summary(result) else {
        return InventoryTransportRoute::UseResult;
    };
    if attempt < max_attempts {
        InventoryTransportRoute::Retry
    } else {
        InventoryTransportRoute::Exhausted(error)
    }
}

fn transport_failure_summary(result: &serde_json::Value) -> Option<String> {
    let status = result
        .get("status")
        .or_else(|| result.pointer("/result/status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if status != "failed" {
        return None;
    }
    let candidates = [
        result.get("summary"),
        result.pointer("/data/error"),
        result.pointer("/result/summary"),
        result.pointer("/result/data/error"),
    ];
    let summary = candidates
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .find(|text| is_transport_failure_text(text))?;
    Some(summary.to_string())
}

fn is_transport_failure_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("agent transport failed")
        || text.contains("reducer transport exhausted")
        || text.contains("no safe compaction boundary")
        || text.contains("reactive subagent compaction failed")
        || text.contains("connection reset")
        || text.contains("connection closed")
        || text.contains("spawn failed")
}

#[allow(clippy::too_many_arguments)]
fn verification_remediation_inventory_source(
    task_universe: &serde_json::Value,
    ready_implementation_items: &[serde_json::Value],
    plan_items: &[serde_json::Value],
    triage: &serde_json::Value,
    actionable: &[serde_json::Value],
    implementation: &[serde_json::Value],
    verification: &[serde_json::Value],
    aggressive: bool,
) -> serde_json::Value {
    let item_limit = if aggressive { 24 } else { 64 };
    serde_json::json!([
        task_universe,
        slim_items(ready_implementation_items, item_limit),
        slim_items(plan_items, item_limit),
        triage,
        slim_items(actionable, item_limit),
        slim_verification_records(implementation, aggressive),
        slim_verification_records(verification, aggressive),
    ])
}

fn slim_items(items: &[serde_json::Value], limit: usize) -> Vec<serde_json::Value> {
    let mut seen = std::collections::BTreeSet::new();
    items
        .iter()
        .filter(|item| {
            let id = item_identity(item);
            seen.insert(strip_check_suffixes(id).to_string())
        })
        .take(limit)
        .cloned()
        .collect()
}

fn slim_verification_records(
    records: &[serde_json::Value],
    aggressive: bool,
) -> Vec<serde_json::Value> {
    let record_limit = if aggressive { 3 } else { 8 };
    let outcome_limit = if aggressive { 24 } else { 64 };
    let start = records.len().saturating_sub(record_limit);
    let mut slim = records[start..]
        .iter()
        .map(|record| slim_evidence_record(record, outcome_limit))
        .collect::<Vec<_>>();
    if start > 0 {
        slim.insert(
            0,
            serde_json::json!({
                "kind": "history_overflow_summary",
                "omitted_record_count": start,
            }),
        );
    }
    slim
}

fn slim_evidence_record(record: &serde_json::Value, outcome_limit: usize) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for key in [
        "kind",
        "implementationWaveIndex",
        "dependencyIteration",
        "verificationRepairAttempt",
        "verificationRemediationAttempt",
    ] {
        if let Some(value) = record.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    let result = record.get("result").unwrap_or(record);
    out.insert(
        "result".to_string(),
        slim_result_with_outcomes(result, outcome_limit),
    );
    serde_json::Value::Object(out)
}

fn slim_result_with_outcomes(
    result: &serde_json::Value,
    outcome_limit: usize,
) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for key in ["status", "summary", "residual_gaps", "task_coverage"] {
        if let Some(value) = result.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    if let Some(data) = result.get("data") {
        out.insert("data".to_string(), slim_result_data(data));
    }
    let outcomes = support::array(result.get("outcomes"));
    if !outcomes.is_empty() {
        let (outcomes, omitted) = collapse_outcome_clones(&outcomes, outcome_limit);
        out.insert("outcomes".to_string(), serde_json::Value::Array(outcomes));
        if omitted > 0 {
            out.insert(
                "omitted_outcome_count".to_string(),
                serde_json::json!(omitted),
            );
        }
    }
    serde_json::Value::Object(out)
}

fn slim_result_data(data: &serde_json::Value) -> serde_json::Value {
    let Some(source) = data.as_object() else {
        return serde_json::Value::Null;
    };
    let mut out = serde_json::Map::new();
    for key in [
        "canonical_task_ids",
        "source_item_id",
        "source_residual_gap_ids",
        "verification_failure_class",
        "verification_failure_next_action",
        "matched_test_check_names",
        "pass_fail_count",
        "error",
    ] {
        if let Some(value) = source.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    serde_json::Value::Object(out)
}

fn collapse_outcome_clones(
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

fn outcome_group_identity(stem: &str, outcome: &serde_json::Value) -> String {
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

fn slim_outcome(outcome: &serde_json::Value) -> serde_json::Value {
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

fn item_identity(value: &serde_json::Value) -> &str {
    value
        .get("item_id")
        .or_else(|| value.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn strip_check_suffixes(mut value: &str) -> &str {
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
            &self.task_universe,
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
    let items: Vec<serde_json::Value> = support::retry_verification_items(contract, &constrained)
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
