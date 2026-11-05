use super::super::*;
use super::*;

#[test]
fn missing_usage_stays_unknown_with_explicit_pricing() {
    let llm = canned_specialist_llm();
    assert_eq!(
        costs::estimate_llm_cost_usd(&llm, "custom-model", None, Some(12)),
        None
    );
    assert_eq!(
        costs::estimate_llm_cost_usd(&llm, "custom-model", Some(12), None),
        None
    );
    assert_eq!(
        costs::estimate_llm_cost_usd(&llm, "custom-model", Some(0), Some(0)),
        Some(0.0)
    );
}
