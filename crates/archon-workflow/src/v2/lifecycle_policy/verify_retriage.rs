use serde_json::Value;

use crate::generated_lifecycle_support as support;
use crate::generated_lifecycle_support::LifecycleContract;

use super::{verify_routing, verify_supersede};

pub fn needs_bounded_retriage(
    contract: &LifecycleContract<'_>,
    verification: &Value,
    triage: &Value,
) -> bool {
    let data = triage_data(triage);
    let routes = verify_routing::triage_routes(triage);
    let superseded = support::array(data.get("superseded_items"));
    let terminal = support::array(data.get("terminal_blockers"));
    if !routes.implementation_failures.is_empty()
        || !routes.retry_items.is_empty()
        || superseded.is_empty()
        || !terminal.is_empty()
    {
        return false;
    }
    verify_supersede::try_supersede_verification(
        contract,
        verification,
        triage,
        "supersede-proof-probe",
    )
    .is_none()
}

pub fn retriage_feedback(verification: &Value, triage: &Value) -> Value {
    let failed = support::non_accepted_outcomes(&support::outcomes_of(verification));
    let failed_ids: Vec<String> = failed.iter().filter_map(outcome_id).collect();
    serde_json::json!({
        "issue": "supersede_unprovable",
        "required_route": "corrected_retry_items",
        "instruction": "Emit repository-search-verified corrected retry items for every stale or zero-match filter failure; do not supersede them.",
        "failed_outcome_ids": failed_ids,
        "failed_outcomes": failed,
        "rejected_triage": triage,
    })
}

fn triage_data(triage: &Value) -> &Value {
    triage
        .get("data")
        .or_else(|| triage.get("result").and_then(|result| result.get("data")))
        .unwrap_or(triage)
}

fn outcome_id(outcome: &Value) -> Option<String> {
    outcome
        .get("item_id")
        .or_else(|| outcome.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}
