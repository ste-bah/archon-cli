use super::{MessageHistory, current_trigger_tokens};

#[test]
fn fresh_message_burst_can_raise_stale_provider_usage() {
    let messages = vec![serde_json::json!({
        "role": "user",
        "content": "x".repeat(600_000),
    })];
    let fresh = crate::agent::autocompact::trigger_tokens(&messages);

    assert_eq!(
        current_trigger_tokens(&MessageHistory::new(messages), 10),
        fresh
    );
}

#[test]
fn five_tool_result_burst_raises_pressure_above_stale_usage() {
    let messages: Vec<serde_json::Value> = (0..5)
        .map(|index| {
            serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": format!("tool-{index}"),
                    "content": "x".repeat(150 * 1024),
                    "is_error": false,
                }],
            })
        })
        .collect();
    let fresh = crate::agent::autocompact::trigger_tokens(&messages);

    assert!(
        fresh > 150_000,
        "five-result burst must create measurable pressure"
    );
    assert_eq!(
        current_trigger_tokens(&MessageHistory::new(messages), 10),
        fresh
    );
}

#[test]
fn provider_usage_can_raise_low_fresh_estimate() {
    let messages = vec![serde_json::json!({"role": "user", "content": "small"})];

    assert_eq!(
        current_trigger_tokens(&MessageHistory::new(messages), 900_000),
        900_000
    );
}

/// #171 parts 1+7: the guard must see the same number whether the estimate
/// arrives incrementally or from a full recount, including after the history
/// is grown one message at a time.
#[test]
fn incremental_history_growth_matches_a_full_recount_trigger() {
    let mut grown = MessageHistory::new(vec![serde_json::json!({
        "role": "user",
        "content": "start",
    })]);
    let mut flat = vec![serde_json::json!({"role": "user", "content": "start"})];
    for index in 0..6 {
        let message = serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": format!("tool-{index}"),
                "content": "y".repeat(40_000),
                "is_error": false,
            }],
        });
        grown.push(message.clone());
        flat.push(message);

        assert_eq!(
            current_trigger_tokens(&grown, 0),
            crate::agent::autocompact::trigger_tokens(&flat)
        );
    }
}

/// #171 part 7: the derived body size must track the envelope plus the
/// running message estimate, and stay a conservative over-estimate of the
/// bytes the old full serialization measured.
#[test]
fn derived_body_bytes_track_the_envelope_plus_running_estimate() {
    let history = MessageHistory::new(vec![
        serde_json::json!({"role": "user", "content": "x".repeat(40_000)}),
        serde_json::json!({"role": "assistant", "content": "y".repeat(10_000)}),
    ]);
    let messages_bytes = serde_json::to_vec(history.as_slice())
        .expect("serialize fixture history")
        .len();

    let derived =
        crate::agent::autocompact::estimated_body_bytes(1_024, history.estimated_tokens());

    assert!(derived >= 1_024 + messages_bytes - 4 * history.as_slice().len());
    assert!(derived <= 1_024 + messages_bytes + 4 * history.as_slice().len());
}
