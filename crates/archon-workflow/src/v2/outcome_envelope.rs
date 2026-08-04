//! Reading the outcome list out of a host-call result, whatever shape it came
//! back in.
//!
//! A generated wave's result reaches the host as one of several envelopes: the
//! raw value, `{ result: … }` from a branch outcome, or `{ data: … }` from a
//! stored call record — and either `outcomes` or `items` inside any of them.
//! The accessor is a property of that envelope vocabulary, which is this
//! crate's, so both readers live off this one definition rather than two:
//! the local host's blocker digest here, and the decomposed-PRD lifecycle in
//! the bin crate.
//!
//! A value with no recognisable outcome list is returned as a single outcome
//! rather than as an empty list. That is deliberate and load-bearing: an empty
//! list reads downstream as "this wave produced nothing to check", which would
//! let a bare result envelope pass a gate unexamined.

use serde_json::Value;

/// Outcome accessor across the host's known direct and merged envelopes.
pub fn outcomes_of(result: &Value) -> Vec<Value> {
    for envelope in known_result_envelopes(result) {
        for key in ["outcomes", "items"] {
            let values = outcome_array(envelope.get(key));
            if !values.is_empty() {
                return values;
            }
        }
    }
    vec![result.clone()]
}

fn known_result_envelopes(result: &Value) -> Vec<&Value> {
    let mut envelopes = vec![result];
    if let Some(inner) = result.get("result") {
        envelopes.push(inner);
    }
    let roots = envelopes.clone();
    for root in roots {
        if let Some(data) = root.get("data") {
            envelopes.push(data);
        }
    }
    envelopes
}

/// A lone object or scalar under `outcomes`/`items` is one outcome, not none —
/// the generated scaffold emits both shapes and the singular form must not be
/// silently dropped.
fn outcome_array(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other.clone()],
    }
}

#[cfg(test)]
mod tests {
    use super::outcomes_of;

    #[test]
    fn direct_outcomes_win_over_nested_envelopes() {
        let value = serde_json::json!({
            "outcomes": [{ "item_id": "a" }],
            "result": { "outcomes": [{ "item_id": "b" }] },
        });
        assert_eq!(outcomes_of(&value)[0]["item_id"], serde_json::json!("a"));
    }

    #[test]
    fn merged_result_and_data_envelopes_are_searched() {
        let value = serde_json::json!({ "result": { "data": { "items": [{ "item_id": "c" }] } } });
        assert_eq!(outcomes_of(&value)[0]["item_id"], serde_json::json!("c"));
    }

    #[test]
    fn an_envelope_without_outcomes_is_itself_one_outcome() {
        let value = serde_json::json!({ "status": "accepted", "summary": "done" });
        let outcomes = outcomes_of(&value);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0], value);
    }

    #[test]
    fn a_singular_outcome_object_is_not_dropped() {
        let value = serde_json::json!({ "outcomes": { "item_id": "d" } });
        assert_eq!(outcomes_of(&value)[0]["item_id"], serde_json::json!("d"));
    }
}
