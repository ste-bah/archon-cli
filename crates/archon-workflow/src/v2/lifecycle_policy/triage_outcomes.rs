//! The triage denominator: which verification outcomes a failure triage is
//! answerable for.
//!
//! Split out of the binary's `workflow_live_v2_lifecycle_verify_triage.rs`,
//! whose remaining body is an `impl LifecycleDriver` and cannot leave the
//! binary. This is the value-level half, and the routing tests that pin the
//! wiring-error denominator assert against it.

use crate::generated_lifecycle_support as support;

pub fn triage_failed_outcomes(verification: &serde_json::Value) -> Vec<serde_json::Value> {
    let has_concrete_outcomes = [
        verification.pointer("/outcomes"),
        verification.pointer("/items"),
        verification.pointer("/data/outcomes"),
        verification.pointer("/data/items"),
        verification.pointer("/result/outcomes"),
        verification.pointer("/result/items"),
        verification.pointer("/result/data/outcomes"),
        verification.pointer("/result/data/items"),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.as_array().is_some_and(|items| !items.is_empty()));
    let failed = if has_concrete_outcomes {
        support::non_accepted_outcomes(&support::outcomes_of(verification))
    } else {
        Vec::new()
    };
    if !failed.is_empty() || support::outcome_accepted_or_noop(verification) {
        return failed;
    }
    vec![serde_json::json!({
        "item_id": "verification-triage-denominator-wiring-error",
        "status": "failed",
        "failure_kind": "triage_denominator_wiring_error",
        "summary": "non-accepted verification reached triage without any extractable concrete outcomes",
        "result": {
            "status": verification.get("status").cloned().unwrap_or(serde_json::Value::Null),
            "summary": verification.get("summary").cloned().unwrap_or(serde_json::Value::Null),
            "residual_gaps": verification.get("residual_gaps").cloned().unwrap_or_else(|| serde_json::json!([])),
        }
    })]
}
