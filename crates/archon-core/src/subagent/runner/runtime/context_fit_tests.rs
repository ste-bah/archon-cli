//! The fit must hold for any history, because it is the last thing between a
//! conversation and a rejected request.
use super::context_fit::fit_messages_to_budget;
use serde_json::{Value, json};

fn text_message(role: &str, bytes: usize) -> Value {
    json!({"role": role, "content": [{"type": "text", "text": "x".repeat(bytes)}]})
}
fn tokens(messages: &[Value]) -> u64 {
    messages
        .iter()
        .map(crate::agent::autocompact::estimate_message_tokens)
        .sum()
}

#[test]
fn a_history_that_already_fits_is_left_exactly_alone() {
    let messages = vec![text_message("user", 100), text_message("assistant", 100)];
    assert!(fit_messages_to_budget(&messages, 10_000).is_none());
}

#[test]
fn the_result_is_under_budget_for_every_history_shape() {
    // Budgets deliberately span "plenty", "tight", and "smaller than one turn".
    for budget in [50u64, 200, 1_000, 5_000] {
        for count in [1usize, 2, 5, 20] {
            let mut messages = vec![json!({"role": "system", "content": "task"})];
            for i in 0..count {
                messages.push(text_message(
                    if i % 2 == 0 { "user" } else { "assistant" },
                    2_000,
                ));
            }
            let Some((fitted, _)) = fit_messages_to_budget(&messages, budget) else {
                continue;
            };
            assert!(
                tokens(&fitted) <= budget.max(1),
                "budget {budget}, {count} turns: {} tokens survived",
                tokens(&fitted)
            );
            assert!(!fitted.is_empty(), "fit emptied the conversation");
        }
    }
}

#[test]
fn the_system_message_and_the_newest_turn_both_survive() {
    let mut messages = vec![json!({"role": "system", "content": "the task"})];
    for _ in 0..10 {
        messages.push(text_message("user", 4_000));
    }
    messages.push(json!({"role": "user", "content": [{"type": "text", "text": "the newest turn"}]}));
    let (fitted, outcome) = fit_messages_to_budget(&messages, 300).expect("should not fit");
    assert_eq!(fitted[0]["role"], "system");
    let rendered = serde_json::to_string(&fitted).unwrap();
    assert!(rendered.contains("the newest turn"), "newest turn was dropped");
    assert!(outcome.dropped_messages > 0);
}

#[test]
fn a_tool_result_never_leads_the_kept_history() {
    // Its `tool_use` is in the assistant turn before it; a dangling pair is a 400.
    let mut messages = vec![json!({"role": "system", "content": "task"})];
    for _ in 0..6 {
        messages.push(text_message("assistant", 3_000));
        messages.push(json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t", "content": "y".repeat(3_000)}
        ]}));
    }
    for budget in [100u64, 400, 900, 2_000] {
        let Some((fitted, _)) = fit_messages_to_budget(&messages, budget) else {
            continue;
        };
        let first_non_system = fitted.iter().find(|m| m["role"] != "system");
        if let Some(message) = first_non_system {
            let leads_with_result = message["content"]
                .as_array()
                .is_some_and(|b| b.iter().any(|x| x["type"] == "tool_result"));
            assert!(
                !leads_with_result,
                "budget {budget}: kept history opens with an orphaned tool_result"
            );
        }
    }
}

#[test]
fn one_oversized_turn_is_truncated_rather_than_dropped() {
    let messages = vec![text_message("user", 200_000)];
    let (fitted, outcome) = fit_messages_to_budget(&messages, 500).expect("should not fit");
    assert_eq!(fitted.len(), 1);
    assert!(outcome.truncated_tail);
    let rendered = serde_json::to_string(&fitted).unwrap();
    assert!(rendered.contains("dropped to fit the context window"), "no marker");
    assert!(tokens(&fitted) <= 500, "{} tokens survived", tokens(&fitted));
}

#[test]
fn an_unknown_or_collapsed_window_must_not_trim_anything() {
    // The caller skips the fit when the window is 0 (model unknown) or at or
    // below the answer reserve. Running unguarded trimmed three conversations
    // that were never oversized, because a default config's reserve equalled
    // the resolved window. This pins the arithmetic the caller relies on.
    use crate::agent::AgentConfig;
    let mut config = AgentConfig::default();
    config.max_tokens = 32_768;
    config.context.output_reserve_tokens = 8_192;
    let reserve = config.response_reserve_tokens();
    assert_eq!(reserve, 32_768);
    for window in [0u64, 1, 8_192, 32_768] {
        assert!(
            window <= reserve,
            "window {window} would wrongly be treated as trustworthy"
        );
    }
    assert!(262_144u64 > reserve, "a real window must pass the guard");
    assert_eq!(262_144u64.saturating_sub(reserve), 229_376);
}
