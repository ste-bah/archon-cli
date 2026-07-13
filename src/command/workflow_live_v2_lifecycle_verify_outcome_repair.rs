use serde_json::Value;

use super::support;

pub(super) fn repairable_contract_outcomes(remediation_wave: &Value) -> Vec<Value> {
    support::non_accepted_outcomes(&support::outcomes_of(remediation_wave))
        .into_iter()
        .filter(is_contract_failure)
        .collect()
}

pub(super) fn merge_repaired_outcomes(
    remediation_wave: &Value,
    followup_wave: Value,
    followup_items: &[Value],
) -> Value {
    super::workflow_live_v2_lifecycle_verify_merge::merge_repair_outcomes(
        remediation_wave,
        followup_wave,
        followup_items,
    )
}

fn is_contract_failure(outcome: &Value) -> bool {
    let failure_kind = outcome
        .get("failure_kind")
        .or_else(|| outcome.pointer("/result/data/failure_kind"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    matches!(
        failure_kind.trim().to_ascii_lowercase().as_str(),
        "contract" | "format" | "complexity" | "code_hygiene"
    )
}
