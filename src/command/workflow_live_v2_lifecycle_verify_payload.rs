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

pub(super) fn uses_verification_slimming(id: &str) -> bool {
    id.starts_with("verification-failure-triage-")
        || id.starts_with("verification-failure-retriage-")
        || (id.starts_with("verification-repair-plan-")
            && !id.starts_with("verification-repair-plan-repair-"))
}

pub(super) fn slim_reducer_source(
    id: &str,
    source: &serde_json::Value,
    aggressive: bool,
) -> serde_json::Value {
    if !uses_verification_slimming(id) {
        return source.clone();
    }
    let values = source.as_array().cloned().unwrap_or_default();
    let item_limit = if aggressive { 16 } else { 48 };
    let at = |index: usize| values.get(index).cloned().unwrap_or(serde_json::Value::Null);
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
        slim_result_with_outcomes(values.get(2).unwrap_or(&serde_json::Value::Null), item_limit),
        slim_verification_records(&support::array(values.get(3)), aggressive),
    ])
}

fn slim_retriage_feedback(value: Option<&serde_json::Value>, limit: usize) -> serde_json::Value {
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

pub(super) fn slim_verification_records(
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
