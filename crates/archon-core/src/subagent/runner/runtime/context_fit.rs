//! Last-resort guarantee that a request fits the window it is sent to.
//!
//! Compaction is the intended mechanism and this is not a substitute for it:
//! summarising preserves meaning, dropping turns does not. But compaction asks
//! the model for a summary, and that call can be late, refused, truncated, or
//! itself too large — and when it is, the oversized request went out anyway.
//!
//! Observed live on 2026-09-08: the authoring agent overflowed six times in two
//! minutes with `262145 tokens > 262144 maximum`, retried the same oversized
//! request each time, and never reached implementation. xAI's own compaction
//! documentation states the rule this violates — compaction "shrinks the
//! conversation; it does not rescue a request that is already over the limit."
//! `xai-org/grok-build` guarantees the fit separately, in
//! `fit_conversation_to_budget`, which this mirrors for our message shape.
use serde_json::Value;

/// Outcome of one fit, for the caller's log line.
pub(super) struct FitOutcome {
    pub dropped_messages: usize,
    pub truncated_tail: bool,
    pub tokens_before: u64,
    pub tokens_after: u64,
}

fn message_tokens(message: &Value) -> u64 {
    crate::agent::autocompact::estimate_message_tokens(message)
}

fn is_system(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("system")
}

/// A message carrying `tool_result` blocks cannot lead the kept suffix: the
/// `tool_use` it answers sits in the assistant turn immediately before it, and
/// strict backends reject the dangling pair with a 400.
fn opens_with_tool_result(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        })
}

/// Shrink `messages` to at most `budget_tokens`, newest-first.
///
/// System messages are kept — they carry the task itself, and a request without
/// them is worse than a truncated one. Everything else is dropped oldest-first
/// in whole turns, never mid-turn, so no `tool_result` is separated from its
/// `tool_use`. If not even the newest turn fits, it is kept and truncated
/// rather than dropped, because a request with no turns has nothing to answer.
pub(super) fn fit_messages_to_budget(
    messages: &[Value],
    budget_tokens: u64,
) -> Option<(Vec<Value>, FitOutcome)> {
    let tokens_before = messages.iter().map(message_tokens).sum::<u64>();
    if tokens_before <= budget_tokens || messages.is_empty() {
        return None;
    }

    let split = messages.iter().take_while(|m| is_system(m)).count();
    let (head, body) = messages.split_at(split);
    let head_tokens = head.iter().map(message_tokens).sum::<u64>();
    let mut remaining = budget_tokens.saturating_sub(head_tokens);

    let mut start = body.len();
    for index in (0..body.len()).rev() {
        let cost = message_tokens(&body[index]);
        if cost > remaining {
            break;
        }
        remaining -= cost;
        start = index;
    }
    while start < body.len() && opens_with_tool_result(&body[start]) {
        start += 1;
    }

    let mut kept: Vec<Value> = head.to_vec();
    let tail_budget = budget_tokens.saturating_sub(head_tokens);
    let truncated_tail = if start < body.len() {
        kept.extend_from_slice(&body[start..]);
        false
    } else if body.is_empty() {
        false
    } else {
        // Nothing whole fits. Keep the newest turn and cut it down — but a
        // `tool_result` cannot stand alone, so its owning assistant turn comes
        // with it and the two share the budget.
        let mut unit_start = body.len() - 1;
        if opens_with_tool_result(&body[unit_start]) && unit_start > 0 {
            unit_start -= 1;
        }
        let unit = &body[unit_start..];
        let share = tail_budget / unit.len() as u64;
        kept.extend(unit.iter().map(|message| shrink_to_fit(message, share)));
        true
    };

    let tokens_after = kept.iter().map(message_tokens).sum::<u64>();
    let kept_len = kept.len();
    Some((
        kept,
        FitOutcome {
            dropped_messages: messages.len().saturating_sub(kept_len),
            truncated_tail,
            tokens_before,
            tokens_after,
        },
    ))
}

/// Shrink one message until the estimator agrees it fits `budget_tokens`.
///
/// Byte arithmetic alone undershot: the JSON envelope (role, block types, ids)
/// is counted by the estimator but is not text we can cut, so a message
/// truncated to "budget × 4 bytes" still measured over budget. Cutting and then
/// re-measuring is the only version that holds for every shape, so that is what
/// this does, halving until it fits or there is nothing left to cut.
fn shrink_to_fit(message: &Value, budget_tokens: u64) -> Value {
    let mut max_bytes = (budget_tokens as usize).saturating_mul(4);
    for _ in 0..12 {
        let candidate = truncate_message(message, max_bytes);
        if message_tokens(&candidate) <= budget_tokens.max(1) || max_bytes == 0 {
            return candidate;
        }
        max_bytes /= 2;
    }
    truncate_message(message, 0)
}

/// Cut every text block in one message down to `max_bytes`, leaving a marker so
/// a reader can tell truncation from an empty result.
fn truncate_message(message: &Value, max_bytes: usize) -> Value {
    let mut message = message.clone();
    match message.get_mut("content") {
        Some(Value::String(text)) => {
            if let Some(cut) = truncate_text(text, max_bytes) {
                *text = cut;
            }
        }
        Some(Value::Array(blocks)) => {
            let share = max_bytes / blocks.len().max(1);
            for block in blocks.iter_mut() {
                for key in ["text", "content", "thinking"] {
                    if let Some(Value::String(text)) = block.get_mut(key)
                        && let Some(cut) = truncate_text(text, share)
                    {
                        *text = cut;
                    }
                }
            }
        }
        _ => {}
    }
    message
}

/// `None` when `text` already fits. The marker is dropped rather than allowed
/// to push the result back over budget when the budget is smaller than it.
fn truncate_text(text: &str, max_bytes: usize) -> Option<String> {
    if text.len() <= max_bytes {
        return None;
    }
    const MARKER_RESERVE: usize = 72;
    let marker_fits = max_bytes > MARKER_RESERVE;
    let keep = if marker_fits {
        max_bytes - MARKER_RESERVE
    } else {
        max_bytes
    };
    let mut end = keep.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let dropped = text.len() - end;
    Some(if marker_fits {
        format!(
            "{}\n[... {dropped} bytes dropped to fit the context window ...]",
            &text[..end]
        )
    } else {
        text[..end].to_string()
    })
}
