use serde_json::Value;

use crate::generated_lifecycle_support as support;

use super::verify_invariants;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryProducer {
    Triage,
    Retriage,
    RepairPlan,
}

impl RetryProducer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Retriage => "retriage",
            Self::RepairPlan => "repair-plan",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RetryConsumptionRoute {
    NotNeeded,
    RunRetries,
}

#[derive(Debug, Default)]
pub struct VerificationTriageRoutes {
    pub implementation_failures: Vec<Value>,
    pub retry_items: Vec<Value>,
    pub superseded_items: Vec<Value>,
    pub terminal_blockers: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct VerificationTriageRoutePlan {
    pub run_retries: bool,
    pub try_supersede: bool,
    pub run_write_remediation: bool,
    pub terminal_blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RemediationInventoryRoute {
    NotNeeded,
    RunWriteRemediation,
    RegenerateInventory,
    Block,
}

const ROUTE_KEY_ALIASES: [(&str, &[&str]); 4] = [
    ("retry_items", &["retry_items", "retryItems"]),
    (
        "implementation_failures",
        &["implementation_failures", "implementationFailures"],
    ),
    ("superseded_items", &["superseded_items", "supersededItems"]),
    (
        "terminal_blockers",
        &["terminal_blockers", "terminalBlockers"],
    ),
];

const ROUTE_CONTAINER_KEYS: [&str; 3] = ["items", "triage", "routes"];

/// Hoist route arrays that reducers nested under known containers (e.g.
/// `data.items.implementation_failures`) into the canonical top-level
/// collections. Consumers only read the canonical collections; without this
/// a nested-but-valid triage reads as empty routes.
pub fn harvest_nested_triage_routes(triage: &Value) -> Value {
    let mut harvested = triage.clone();
    let Some(data) = data_object_mut(&mut harvested) else {
        return harvested;
    };
    let mut hoisted: Vec<(String, Vec<Value>)> = Vec::new();
    for (canonical, aliases) in ROUTE_KEY_ALIASES {
        let found = aliases
            .iter()
            .flat_map(|alias| support::array(data.get(*alias)))
            .collect::<Vec<_>>();
        if !found.is_empty() {
            hoisted.push((canonical.to_string(), found));
        }
    }
    for container_key in ROUTE_CONTAINER_KEYS {
        let Some(container) = data.get(container_key).filter(|value| value.is_object()) else {
            continue;
        };
        for (canonical, aliases) in ROUTE_KEY_ALIASES {
            let mut found: Vec<Value> = Vec::new();
            for alias in aliases {
                found.extend(support::array(container.get(*alias)));
            }
            if !found.is_empty() {
                hoisted.push((canonical.to_string(), found));
            }
        }
    }
    for (canonical, found) in hoisted {
        let mut merged = support::array(data.get(canonical.as_str()));
        merged.extend(found);
        dedup_items(&mut merged);
        data.insert(canonical, Value::Array(merged));
    }
    harvested
}

/// Non-accepted verification outcomes that no canonical triage route array
/// accounts for. A triage leaving failures unaccounted is a shape failure to
/// repair, never an empty result to consume.
pub fn unaccounted_failed_outcomes(triage: &Value, failed_outcomes: &[Value]) -> Vec<Value> {
    let routes = triage_routes(triage);
    let mut routed_ids: std::collections::BTreeSet<String> = Default::default();
    for items in [
        &routes.retry_items,
        &routes.implementation_failures,
        &routes.superseded_items,
        &routes.terminal_blockers,
    ] {
        for item in items {
            routed_ids.extend(verify_invariants::verification_item_ids(item));
        }
    }
    failed_outcomes
        .iter()
        .filter(|outcome| {
            verify_invariants::verification_item_ids(outcome)
                .iter()
                .all(|id| !routed_ids.contains(id))
        })
        .cloned()
        .collect()
}

fn data_object_mut(triage: &mut Value) -> Option<&mut serde_json::Map<String, Value>> {
    if triage.get("data").is_some() {
        return triage.get_mut("data").and_then(Value::as_object_mut);
    }
    if triage
        .get("result")
        .and_then(|result| result.get("data"))
        .is_some()
    {
        return triage
            .get_mut("result")
            .and_then(|result| result.get_mut("data"))
            .and_then(Value::as_object_mut);
    }
    triage.as_object_mut()
}

pub fn triage_routes(triage: &Value) -> VerificationTriageRoutes {
    let data = triage_data(triage);
    let mut implementation_failures = support::array(data.get("implementation_failures"));
    implementation_failures.extend(support::array(data.get("implementationFailures")));
    implementation_failures.extend(
        support::array(data.get("items"))
            .into_iter()
            .filter(is_actionable_classification),
    );
    dedup_items(&mut implementation_failures);
    let retry_items = retry_items(triage);
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

pub fn retry_items(producer_output: &Value) -> Vec<Value> {
    let data = triage_data(producer_output);
    let mut retry_items = support::array(data.get("retry_items"));
    retry_items.extend(support::array(data.get("retryItems")));
    dedup_items(&mut retry_items);
    retry_items
}

pub fn retry_consumption_route(
    _producer: RetryProducer,
    retry_items: &[Value],
) -> RetryConsumptionRoute {
    if retry_items.is_empty() {
        RetryConsumptionRoute::NotNeeded
    } else {
        RetryConsumptionRoute::RunRetries
    }
}

pub fn triage_route_plan(routes: &VerificationTriageRoutes) -> VerificationTriageRoutePlan {
    let run_retries = !routes.retry_items.is_empty();
    let try_supersede =
        !routes.superseded_items.is_empty() || routes.retry_items.iter().any(is_sibling_resolved);
    let run_write_remediation = !routes.implementation_failures.is_empty();
    let independent_terminal_blocker = routes
        .terminal_blockers
        .iter()
        .any(|blocker| terminal_blocker_is_independent(blocker, &routes.retry_items));
    VerificationTriageRoutePlan {
        run_retries,
        try_supersede,
        run_write_remediation,
        terminal_blocked: independent_terminal_blocker
            || (!routes.terminal_blockers.is_empty()
                && !run_retries
                && !try_supersede
                && !run_write_remediation),
    }
}

fn terminal_blocker_is_independent(blocker: &Value, retry_items: &[Value]) -> bool {
    let affected = blocker
        .get("affected_retry_items")
        .or_else(|| blocker.get("affectedRetryItems"))
        .map(|value| support::strings_of(Some(value)))
        .unwrap_or_default();
    if affected.is_empty() {
        return true;
    }
    let retry_ids = retry_items
        .iter()
        .flat_map(|item| {
            ["item_id", "source_item_id"]
                .into_iter()
                .filter_map(|key| item.get(key).and_then(Value::as_str))
        })
        .collect::<std::collections::BTreeSet<_>>();
    !affected
        .iter()
        .any(|item_id| retry_ids.contains(item_id.as_str()))
}

pub fn remediation_inventory_route(
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

pub fn write_remediation_outcomes(repair_plan: &Value, verification: &Value) -> Vec<Value> {
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

pub fn repeated_gap_write_remediation_outcomes(
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

pub fn predicate_rewrite_inventory(repair_plan: &Value, verification: &Value) -> Option<Value> {
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
#[path = "verify_routing_tests.rs"]
mod tests;
