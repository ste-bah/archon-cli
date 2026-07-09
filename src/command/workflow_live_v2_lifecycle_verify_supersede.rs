use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;
use crate::command::workflow_live::workflow_live_generated_lifecycle_support::LifecycleContract;

pub(super) struct VerificationSupersede {
    pub(super) verification: Value,
    pub(super) record: Value,
}

pub(super) fn try_supersede_verification(
    contract: &LifecycleContract<'_>,
    verification: &Value,
    triage: &Value,
    triage_call_id: &str,
) -> Option<VerificationSupersede> {
    if triage_has_blockers(triage) {
        return None;
    }
    let outcomes = support::outcomes_of(verification);
    let failed = support::non_accepted_outcomes(&outcomes);
    if failed.is_empty() {
        return None;
    }
    let accepted: Vec<Value> = outcomes
        .iter()
        .filter(|outcome| support::outcome_accepted_or_noop(outcome))
        .cloned()
        .collect();
    let records = supersede_records(contract, &failed, &accepted, triage)?;
    Some(VerificationSupersede {
        verification: verification_with_supersede(verification, &records),
        record: serde_json::json!({
            "kind": "verification-supersede",
            "triage_call_id": triage_call_id,
            "reason": "triage found no implementation or terminal blocker and accepted sibling evidence covers the failed verifier shape",
            "superseded": records,
        }),
    })
}

fn triage_has_blockers(triage: &Value) -> bool {
    let data = triage_data(triage);
    !support::array(data.and_then(|data| data.get("implementation_failures"))).is_empty()
        || !support::array(data.and_then(|data| data.get("terminal_blockers"))).is_empty()
}

fn supersede_records(
    contract: &LifecycleContract<'_>,
    failed: &[Value],
    accepted: &[Value],
    triage: &Value,
) -> Option<Vec<Value>> {
    let mut records = Vec::new();
    for failure in failed {
        if !triage_marks_shape_or_resolved(failure, triage) {
            return None;
        }
        let siblings = accepted_sibling_ids(contract, failure, accepted);
        if siblings.is_empty() {
            return None;
        }
        records.push(serde_json::json!({
            "failed_outcome_id": outcome_id(failure),
            "adopted_accepted_sibling_ids": siblings,
            "canonical_task_ids": contract.canonical_ids_for(failure),
        }));
    }
    Some(records)
}

fn triage_marks_shape_or_resolved(failure: &Value, triage: &Value) -> bool {
    let failure_ids = outcome_match_ids(failure);
    triage_items(triage).iter().any(|item| {
        let class = item
            .get("classification")
            .or_else(|| item.get("verification_failure_class"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        (class.contains("shape") || class.contains("resolved"))
            && outcome_match_ids(item)
                .iter()
                .any(|id| failure_ids.contains(id))
    })
}

fn accepted_sibling_ids(
    contract: &LifecycleContract<'_>,
    failure: &Value,
    accepted: &[Value],
) -> Vec<String> {
    let failed_tasks = contract.canonical_ids_for(failure);
    accepted
        .iter()
        .filter(|outcome| {
            !contract.canonical_ids_for(outcome).is_empty()
                && contract
                    .canonical_ids_for(outcome)
                    .iter()
                    .any(|id| failed_tasks.contains(id))
        })
        .map(outcome_id)
        .collect()
}

fn verification_with_supersede(verification: &Value, records: &[Value]) -> Value {
    let mut object = verification.as_object().cloned().unwrap_or_default();
    object.insert("status".to_string(), Value::String("accepted".to_string()));
    object.insert(
        "superseded_verification_outcomes".to_string(),
        Value::Array(records.to_vec()),
    );
    object.insert("residual_gaps".to_string(), Value::Array(Vec::new()));
    Value::Object(object)
}

fn triage_items(triage: &Value) -> Vec<Value> {
    let Some(data) = triage_data(triage) else {
        return Vec::new();
    };
    let mut items = support::array(data.get("retry_items"));
    items.extend(support::array(data.get("retryItems")));
    items.extend(support::array(data.get("superseded_items")));
    items.extend(support::array(data.get("supersededItems")));
    items
}

fn triage_data(triage: &Value) -> Option<&Value> {
    triage
        .get("data")
        .or_else(|| triage.get("result").and_then(|result| result.get("data")))
}

fn outcome_match_ids(value: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["item_id", "id", "source_item_id", "split_from_item_id"] {
        if let Some(id) = value.get(key).and_then(Value::as_str) {
            ids.push(id.to_string());
        }
    }
    ids.extend(support::strings_of(value.get("source_outcome_item_ids")));
    ids
}

fn outcome_id(value: &Value) -> String {
    outcome_match_ids(value)
        .into_iter()
        .next()
        .unwrap_or_else(|| "verification-outcome".to_string())
}
