//! Mechanical pruning re-judges request pressure from the pruned history
//! (operator decision 2026-10-13, the #369 follow-up): the provider's measured
//! count describes the request BEFORE the prune, so reusing it after a prune
//! could never clear token pressure and LLM compaction always followed.
use super::super::message_history::MessageHistory;
use super::{PruneVerdict, prune_then_judge_pressure};
use crate::config::ContextConfig;

fn context(tokens: u64) -> ContextConfig {
    ContextConfig {
        rate_limit_pressure_tokens: Some(tokens),
        rate_limit_pressure_body_bytes: None,
        ..ContextConfig::default()
    }
}

/// A one-message history whose estimate is about `tokens`.
fn history_of(tokens: u64) -> MessageHistory {
    MessageHistory::new(vec![serde_json::json!({
        "role": "user",
        "content": "x".repeat((tokens * 4) as usize),
    })])
}

fn body_bytes(history: &MessageHistory) -> usize {
    crate::agent::autocompact::estimated_body_bytes(0, history.estimated_tokens())
}

/// (a) Measured 270K, pruning brings the estimate to ~200K: the request is
/// under the 256K limit, so no LLM compaction; the measured count is lowered
/// by what the prune reclaimed, so the pre-prune count cannot re-raise
/// pressure before the next response.
#[test]
fn pruning_under_the_limit_clears_pressure_without_llm_compaction() {
    let config = context(256_000);
    let mut messages = history_of(300_000);
    let mut last_known = 270_000;
    let mut body = body_bytes(&messages);
    let before = messages.estimated_tokens();
    let verdict =
        prune_then_judge_pressure(&config, &mut messages, &mut last_known, &mut body, 0, |m| {
            let before = m.estimated_tokens();
            m.replace(history_of(200_000).as_slice().to_vec());
            before - m.estimated_tokens()
        });
    let reclaimed = before - messages.estimated_tokens();
    assert!(reclaimed > 99_000, "{reclaimed}");
    assert_eq!(verdict, PruneVerdict::Cleared);
    assert_eq!(
        last_known,
        270_000 - reclaimed,
        "lowered by what the prune reclaimed"
    );
    assert!(super::current_trigger_tokens(&messages, last_known) < 256_000);
    assert_eq!(
        body,
        body_bytes(&messages),
        "body size is re-derived from the pruned history"
    );
}

/// (b) Pruning that does not bring the request under the limit leaves it
/// over: LLM compaction follows.
#[test]
fn pruning_not_enough_still_needs_llm_compaction() {
    let config = context(256_000);
    let mut messages = history_of(320_000);
    let mut last_known = 330_000;
    let mut body = body_bytes(&messages);
    let verdict =
        prune_then_judge_pressure(&config, &mut messages, &mut last_known, &mut body, 0, |m| {
            let before = m.estimated_tokens();
            m.replace(history_of(280_000).as_slice().to_vec());
            before - m.estimated_tokens()
        });
    assert_eq!(verdict, PruneVerdict::StillOver);
}

/// Nothing pruned: the measured count is kept and decides, as before.
#[test]
fn nothing_pruned_keeps_the_measured_count() {
    let config = context(256_000);
    let mut messages = history_of(1_000);
    let mut last_known = 270_000;
    let mut body = body_bytes(&messages);
    let verdict =
        prune_then_judge_pressure(&config, &mut messages, &mut last_known, &mut body, 0, |_| 0);
    assert_eq!(verdict, PruneVerdict::NothingPruned);
    assert_eq!(last_known, 270_000);
    assert!(super::current_trigger_tokens(&messages, last_known) >= 256_000);
}

/// Body-byte pressure still counts after a prune that clears token pressure.
#[test]
fn body_pressure_after_the_prune_still_needs_llm_compaction() {
    let mut config = context(256_000);
    config.rate_limit_pressure_body_bytes = Some(100);
    let mut messages = history_of(300_000);
    let mut last_known = 270_000;
    let mut body = body_bytes(&messages);
    let verdict =
        prune_then_judge_pressure(&config, &mut messages, &mut last_known, &mut body, 0, |m| {
            let before = m.estimated_tokens();
            m.replace(history_of(200_000).as_slice().to_vec());
            before - m.estimated_tokens()
        });
    assert_eq!(verdict, PruneVerdict::StillOver);
}

/// (c) The next response's real count takes over again: the run loop assigns
/// it (`last_known_context_tokens = stream.context_input_tokens`), and the
/// trigger uses it even when the pruned estimate is lower.
#[test]
fn the_next_measured_count_is_used_again() {
    let config = context(256_000);
    let mut messages = history_of(300_000);
    let mut last_known = 270_000;
    let mut body = body_bytes(&messages);
    prune_then_judge_pressure(&config, &mut messages, &mut last_known, &mut body, 0, |m| {
        let before = m.estimated_tokens();
        m.replace(history_of(200_000).as_slice().to_vec());
        before - m.estimated_tokens()
    });
    assert!(last_known < 256_000);
    last_known = 262_000; // the provider's count for the pruned request
    assert_eq!(
        super::current_trigger_tokens(&messages, last_known),
        262_000
    );
}

/// A prune that reclaims little does not pass a request the provider still
/// counts over the limit: the measured part no estimate sees (system prompt,
/// tool schemas) is kept. Measured 270K, history estimate ~250K, prune 10K:
/// the pruned estimate (~240K) is under 256K but the measured count less the
/// reclaimed 10K (~260K) is not.
#[test]
fn a_small_prune_keeps_the_unestimated_part_of_the_measured_count() {
    let config = context(256_000);
    let mut messages = history_of(250_000);
    let mut last_known = 270_000;
    let mut body = body_bytes(&messages);
    let verdict =
        prune_then_judge_pressure(&config, &mut messages, &mut last_known, &mut body, 0, |m| {
            let before = m.estimated_tokens();
            m.replace(history_of(240_000).as_slice().to_vec());
            before - m.estimated_tokens()
        });
    assert!(messages.estimated_tokens() < 256_000);
    assert_eq!(verdict, PruneVerdict::StillOver);
    assert!(last_known >= 256_000, "{last_known}");
}

/// Re-review of the #369 follow-up: the provider counts the PROJECTED request,
/// in which tool results older than `preserve_recent_turns` are already cut
/// to their context budget (64 KB by default). A prune that merges such an
/// old result reclaims far more of the stored history than of what the
/// provider counted, so the measured count may only be lowered by what the
/// projection loses. The reviewer's input: measured 300,000; history estimate
/// about 250,000; limit 256,000; one old result stored at 256,000 bytes.
#[test]
fn pruning_an_old_result_the_projection_already_cut_stays_over_the_limit() {
    let config = context(256_000);
    let big = "r".repeat(256_000);
    let filler = "f".repeat(740_000);
    let mut stored = vec![
        serde_json::json!({"role": "user", "content": "task"}),
        serde_json::json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "old-1", "name": "Read", "input": {"path": "a"}}
        ]}),
        serde_json::json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "old-1", "content": big}
        ]}),
    ];
    // Three recent user turns: everything after the first is preserved.
    for turn in 0..3 {
        stored.push(serde_json::json!({"role": "user", "content": format!("turn {turn} {}", &filler[..filler.len() / 3])}));
        stored.push(serde_json::json!({"role": "assistant", "content": "ok"}));
    }
    let mut messages = MessageHistory::new(stored);
    let estimate_before = messages.estimated_tokens();
    assert!(
        (240_000..256_000).contains(&estimate_before),
        "{estimate_before}"
    );
    let mut last_known = 300_000;
    let mut body = body_bytes(&messages);
    let verdict = prune_then_judge_pressure(
        &config,
        &mut messages,
        &mut last_known,
        &mut body,
        0,
        |m| {
            let before = m.estimated_tokens();
            let mut pruned = m.as_slice().to_vec();
            pruned[2] = serde_json::json!({"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "old-1", "content": "[superseded: see later read]"}
            ]});
            m.replace(pruned);
            before - m.estimated_tokens()
        },
    );
    // Stored reclaim is ~64,000 tokens; the provider only ever saw ~16,000.
    assert!(estimate_before - messages.estimated_tokens() > 60_000);
    println!(
        "stored reclaim {}, lowered measured count {last_known}",
        estimate_before - messages.estimated_tokens()
    );
    assert_eq!(verdict, PruneVerdict::StillOver);
    assert!(last_known >= 280_000, "{last_known}");
}
