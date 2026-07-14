use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

#[derive(Debug, Default)]
pub(super) struct VerificationTriageRoutes {
    pub(super) implementation_failures: Vec<Value>,
    pub(super) retry_items: Vec<Value>,
    pub(super) superseded_items: Vec<Value>,
    pub(super) terminal_blockers: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct VerificationTriageRoutePlan {
    pub(super) run_retries: bool,
    pub(super) try_supersede: bool,
    pub(super) run_write_remediation: bool,
    pub(super) terminal_blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RemediationInventoryRoute {
    NotNeeded,
    RunWriteRemediation,
    RegenerateInventory,
    Block,
}

pub(super) fn triage_routes(triage: &Value) -> VerificationTriageRoutes {
    let data = triage_data(triage);
    let mut implementation_failures = support::array(data.get("implementation_failures"));
    implementation_failures.extend(
        support::array(data.get("items"))
            .into_iter()
            .filter(is_actionable_classification),
    );
    dedup_items(&mut implementation_failures);
    let mut retry_items = support::array(data.get("retry_items"));
    retry_items.extend(support::array(data.get("retryItems")));
    dedup_items(&mut retry_items);
    let mut superseded_items = support::array(data.get("superseded_items"));
    superseded_items.extend(support::array(data.get("supersededItems")));
    dedup_items(&mut superseded_items);
    let mut terminal_blockers = support::array(data.get("terminal_blockers"));
    terminal_blockers.extend(support::array(data.get("terminalBlockers")));
    dedup_items(&mut terminal_blockers);
    VerificationTriageRoutes {
        implementation_failures,
        retry_items,
        superseded_items,
        terminal_blockers,
    }
}

pub(super) fn triage_route_plan(routes: &VerificationTriageRoutes) -> VerificationTriageRoutePlan {
    VerificationTriageRoutePlan {
        run_retries: !routes.retry_items.is_empty(),
        try_supersede: !routes.superseded_items.is_empty()
            || routes.retry_items.iter().any(is_sibling_resolved),
        run_write_remediation: !routes.implementation_failures.is_empty(),
        terminal_blocked: !routes.terminal_blockers.is_empty(),
    }
}

pub(super) fn remediation_inventory_route(
    plan: &VerificationTriageRoutePlan,
    inventory_ready: bool,
) -> RemediationInventoryRoute {
    if plan.terminal_blocked {
        return RemediationInventoryRoute::Block;
    }
    if !plan.run_write_remediation {
        return RemediationInventoryRoute::NotNeeded;
    }
    if inventory_ready {
        RemediationInventoryRoute::RunWriteRemediation
    } else {
        RemediationInventoryRoute::RegenerateInventory
    }
}

pub(super) fn write_remediation_outcomes(repair_plan: &Value, verification: &Value) -> Vec<Value> {
    let data = repair_plan
        .get("data")
        .or_else(|| {
            repair_plan
                .get("result")
                .and_then(|result| result.get("data"))
        })
        .unwrap_or(repair_plan);
    if data.get("route").and_then(Value::as_str) != Some("write_remediation") {
        return Vec::new();
    }
    let requested = route_outcome_ids(data);
    support::non_accepted_outcomes(&support::outcomes_of(verification))
        .into_iter()
        .filter(|outcome| requested.is_empty() || requested.contains(&outcome_id(outcome)))
        .collect()
}

pub(super) fn repeated_gap_write_remediation_outcomes(
    verification_history: &[Value],
    verification: &Value,
) -> Vec<Value> {
    let mut generations = std::collections::BTreeMap::new();
    for record in verification_history
        .iter()
        .filter(|record| is_retry(record))
    {
        for gap_id in retry_generation_gap_ids(record) {
            *generations.entry(gap_id).or_insert(0usize) += 1;
        }
    }
    support::non_accepted_outcomes(&support::outcomes_of(verification))
        .into_iter()
        .filter(|outcome| {
            outcome_gap_ids(outcome)
                .iter()
                .any(|id| generations.get(id).is_some_and(|count| *count >= 2))
        })
        .collect()
}

fn is_retry(record: &Value) -> bool {
    matches!(
        record.get("kind").and_then(Value::as_str),
        Some("verification-retry" | "verification-triage-retry")
    )
}

fn retry_generation_gap_ids(record: &Value) -> std::collections::BTreeSet<String> {
    let result = record.get("result").unwrap_or(record);
    let retried_ids = support::array(
        record
            .get("verificationPlan")
            .and_then(|plan| plan.get("items")),
    )
    .iter()
    .flat_map(|item| {
        ["item_id", "id", "source_item_id"]
            .iter()
            .filter_map(|key| item.get(*key).and_then(Value::as_str).map(str::to_string))
    })
    .collect::<std::collections::BTreeSet<_>>();
    support::non_accepted_outcomes(&support::outcomes_of(result))
        .iter()
        .filter(|outcome| outcome_matches_retry(outcome, &retried_ids))
        .flat_map(outcome_gap_ids)
        .collect()
}

fn outcome_matches_retry(
    outcome: &Value,
    retried_ids: &std::collections::BTreeSet<String>,
) -> bool {
    if retried_ids.is_empty() {
        return true;
    }
    let id = outcome_id(outcome);
    retried_ids
        .iter()
        .any(|retry_id| id == *retry_id || id.ends_with(&format!("-{retry_id}")))
}

fn outcome_gap_ids(outcome: &Value) -> std::collections::BTreeSet<String> {
    let result = outcome.get("result").unwrap_or(outcome);
    let data = result.get("data").unwrap_or(&Value::Null);
    let mut ids: std::collections::BTreeSet<String> =
        support::strings_of(data.get("source_residual_gap_ids"))
            .into_iter()
            .collect();
    for gap in support::array(result.get("residual_gaps")) {
        if let Some(id) = gap.get("id").and_then(Value::as_str) {
            ids.insert(id.to_string());
        }
    }
    ids
}

pub(super) fn predicate_rewrite_inventory(
    repair_plan: &Value,
    verification: &Value,
) -> Option<Value> {
    let data = triage_data(repair_plan);
    if data.get("route").and_then(Value::as_str) != Some("predicate_unsatisfiable_as_written") {
        return None;
    }
    let mut items = support::array(data.get("re_authored_items"));
    if items.is_empty() {
        items = support::array(data.get("items"));
    }
    let outcomes = support::non_accepted_outcomes(&support::outcomes_of(verification));
    for item in &mut items {
        stamp_failed_predicate(item, &outcomes);
    }
    Some(serde_json::json!({ "status": "accepted", "items": items }))
}

fn stamp_failed_predicate(item: &mut Value, outcomes: &[Value]) {
    let source_id = item
        .get("source_item_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(outcome) = outcomes.iter().find(|outcome| {
        let id = outcome_id(outcome);
        !source_id.is_empty() && (id == source_id || id.ends_with(&format!("-{source_id}")))
    }) else {
        return;
    };
    let gaps = residual_gaps(outcome);
    let ids: Vec<String> = gaps
        .iter()
        .filter_map(|gap| gap.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let predicate = gaps
        .first()
        .and_then(|gap| gap.get("description"))
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(object) = item.as_object_mut() {
        object.insert(
            "source_residual_gap_ids".to_string(),
            serde_json::json!(ids),
        );
        object.insert("failed_predicate".to_string(), predicate);
    }
}

fn residual_gaps(outcome: &Value) -> Vec<Value> {
    let result = outcome.get("result").unwrap_or(outcome);
    support::array(result.get("residual_gaps"))
}

fn route_outcome_ids(data: &Value) -> Vec<String> {
    [
        "source_outcome_ids",
        "affected_source_outcome_ids",
        "outcome_ids",
    ]
    .iter()
    .flat_map(|key| support::strings_of(data.get(*key)))
    .collect()
}

fn outcome_id(outcome: &Value) -> String {
    outcome
        .get("item_id")
        .or_else(|| outcome.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn triage_data(triage: &Value) -> &Value {
    triage
        .get("data")
        .or_else(|| triage.get("result").and_then(|result| result.get("data")))
        .unwrap_or(triage)
}

fn is_actionable_classification(item: &Value) -> bool {
    let class = item
        .get("classification")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    class.contains("actionable") || class.contains("implementation_failure")
}

fn is_sibling_resolved(item: &Value) -> bool {
    item.get("classification")
        .or_else(|| item.get("verification_failure_class"))
        .and_then(Value::as_str)
        .is_some_and(|class| class.to_ascii_lowercase().contains("sibling"))
}

fn dedup_items(items: &mut Vec<Value>) {
    let mut seen = std::collections::BTreeSet::new();
    items.retain(|item| seen.insert(item.to_string()));
}

#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_verify_routing_tests.rs"]
mod tests;
