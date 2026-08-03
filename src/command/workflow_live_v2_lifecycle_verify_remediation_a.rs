use super::*;

impl LifecycleDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_write_verification_remediation(
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
        let mut routes = lifecycle_policy::verify_routing::triage_routes(triage);
        if routes.implementation_failures.is_empty() {
            routes.implementation_failures = actionable.to_vec();
        }
        let route_plan = lifecycle_policy::verify_routing::triage_route_plan(&routes);
        match lifecycle_policy::verify_routing::remediation_inventory_route(
            &route_plan,
            remediation::remediation_inventory_ready(&remediation_inventory),
        ) {
            lifecycle_policy::verify_routing::RemediationInventoryRoute::RunWriteRemediation => {}
            lifecycle_policy::verify_routing::RemediationInventoryRoute::RegenerateInventory => {
                return Ok(true);
            }
            lifecycle_policy::verify_routing::RemediationInventoryRoute::NotNeeded
            | lifecycle_policy::verify_routing::RemediationInventoryRoute::Block => {
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
    pub(super) async fn verification_remediation_inventory(
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
                    transport_failure_result(
                        &call_id,
                        transport_attempt,
                        max_transport_attempts,
                        &error.to_string(),
                    )
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

pub(crate) fn transport_failure_result(
    call_id: &str,
    attempts: usize,
    max_attempts: usize,
    error: &str,
) -> serde_json::Value {
    // The terminal gate's transport_retry_budget work-kind fires only when it
    // can see attempts < max_transport_attempts on one object — both fields
    // must travel together or the budget is dead-wired.
    serde_json::json!({
        "status": "failed",
        "summary": format!(
            "reducer transport failed at attempt {attempts} of {max_attempts} for '{call_id}': {error}"
        ),
        "data": {
            "error": error,
            "failure_class": "transport_infrastructure",
            "transport_exhausted": attempts >= max_attempts,
            "transport_attempts": attempts,
            "max_transport_attempts": max_attempts,
            "terminal_blockers": [{
                "id": format!("transport-exhausted-{call_id}"),
                "classification": "transport_infrastructure_exhausted",
                "description": error,
                "call_id": call_id,
                "attempts": attempts,
                "max_transport_attempts": max_attempts,
            }],
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InventoryTransportRoute {
    UseResult,
    Retry,
    Exhausted(String),
}

pub(crate) fn inventory_transport_route(
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

pub(crate) fn transport_failure_summary(result: &serde_json::Value) -> Option<String> {
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

pub(crate) fn is_transport_failure_text(text: &str) -> bool {
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
pub(super) fn verification_remediation_inventory_source(
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

pub(crate) fn uses_verification_slimming(id: &str) -> bool {
    id.starts_with("verification-failure-triage-")
        || id.starts_with("verification-failure-retriage-")
        || (id.starts_with("verification-repair-plan-")
            && !id.starts_with("verification-repair-plan-repair-"))
}

pub(crate) fn slim_reducer_source(
    id: &str,
    source: &serde_json::Value,
    aggressive: bool,
) -> serde_json::Value {
    if !uses_verification_slimming(id) {
        return source.clone();
    }
    let values = source.as_array().cloned().unwrap_or_default();
    let item_limit = if aggressive { 16 } else { 48 };
    let at = |index: usize| {
        values
            .get(index)
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    if id.starts_with("verification-failure-triage-") {
        return serde_json::json!([
            at(0),
            slim_items(&support::array(values.get(1)), item_limit),
            slim_items(&support::array(values.get(2)), item_limit),
            slim_items(&support::array(values.get(3)), item_limit),
            slim_verification_records(&support::array(values.get(4)), aggressive),
            slim_verification_records(&support::array(values.get(5)), aggressive),
        ]);
    }
    if id.starts_with("verification-failure-retriage-") {
        return serde_json::json!([
            at(0),
            slim_items(&support::array(values.get(1)), item_limit),
            slim_items(&support::array(values.get(2)), item_limit),
            slim_retriage_feedback(values.get(3), item_limit),
        ]);
    }
    serde_json::json!([
        at(0),
        slim_items(&support::array(values.get(1)), item_limit),
        slim_result_with_outcomes(
            values.get(2).unwrap_or(&serde_json::Value::Null),
            item_limit
        ),
        slim_verification_records(&support::array(values.get(3)), aggressive),
    ])
}

pub(super) fn slim_retriage_feedback(
    value: Option<&serde_json::Value>,
    limit: usize,
) -> serde_json::Value {
    let Some(value) = value.and_then(serde_json::Value::as_object) else {
        return serde_json::Value::Null;
    };
    let mut out = serde_json::Map::new();
    for key in ["issue", "required_route", "instruction"] {
        if let Some(value) = value.get(key) {
            out.insert(key.to_string(), value.clone());
        }
    }
    out.insert(
        "failed_outcome_ids".to_string(),
        serde_json::Value::Array(
            support::array(value.get("failed_outcome_ids"))
                .into_iter()
                .take(limit)
                .collect(),
        ),
    );
    out.insert(
        "failed_outcomes".to_string(),
        serde_json::Value::Array(
            support::array(value.get("failed_outcomes"))
                .iter()
                .take(limit)
                .map(slim_outcome)
                .collect(),
        ),
    );
    if let Some(triage) = value.get("rejected_triage") {
        out.insert(
            "rejected_triage".to_string(),
            slim_result_with_outcomes(triage, limit),
        );
    }
    serde_json::Value::Object(out)
}

pub(super) fn slim_items(items: &[serde_json::Value], limit: usize) -> Vec<serde_json::Value> {
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

pub(crate) fn slim_verification_records(
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

pub(super) fn slim_evidence_record(
    record: &serde_json::Value,
    outcome_limit: usize,
) -> serde_json::Value {
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

pub(super) fn slim_result_with_outcomes(
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

pub(super) fn slim_result_data(data: &serde_json::Value) -> serde_json::Value {
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
