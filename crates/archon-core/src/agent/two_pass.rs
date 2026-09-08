//! Hierarchical compaction: squeeze the old history hard, then merge it with
//! the recent turns.
//!
//! A single pass spreads its attention evenly over the whole conversation, so
//! the recent work — the part a successor actually needs to carry on — gets the
//! same treatment as something from hours ago, and is the first thing lost when
//! the budget is tight. Two passes compress the old bulk first and then give
//! the recent turns proper room alongside that compressed note.
//!
//! Mirrors `xai-org/grok-build`'s `two_pass.rs`, adapted to our message shape.
//! Pass 1 covers ~95% of the history by estimated token weight; the remaining
//! ~5% tail is what pass 2 rewrites against.
use serde_json::{Value, json};

/// Prefix share of the history, by estimated tokens. The tail is the rest, and
/// it stays small because pass 2 pays for it in full on the blocking path.
pub(super) const DEFAULT_SPLIT_FRACTION: f64 = 0.95;

/// Below this there is nothing to gain from splitting — pass 1 would summarise
/// almost nothing and we would pay two model calls for one pass's worth of work.
pub(super) const MIN_MESSAGES_FOR_TWO_PASS: usize = 8;

/// Estimated input tokens below which one pass is the better trade.
///
/// Two passes cost two blocking model calls. `grok-build` can afford that on
/// every compaction because their pass 1 runs in the background ahead of the
/// threshold; ours does not, so both calls sit on the path the agent is waiting
/// on — and compaction being slow is part of what this work exists to fix.
///
/// A history that fits comfortably in one summary does not need splitting: the
/// even spread across old and recent turns only starts costing recent detail
/// once the summary budget is the binding constraint. 60k is several times the
/// largest summary we will ask for (16384) and well under the input cap, so
/// small compactions stay one call and large ones get the hierarchy.
pub(super) const MIN_TOKENS_FOR_TWO_PASS: u64 = 60_000;

/// Cap on the pass-1 note carried into pass 2, so a long note cannot crowd out
/// the recent turns it is supposed to be merged with.
const MAX_NOTE_CHARS: usize = 12_000;

pub(super) struct Split<'a> {
    pub prefix: &'a [Value],
    pub tail: &'a [Value],
}

fn message_tokens(message: &Value) -> u64 {
    super::autocompact::estimate_message_tokens(message)
}

fn is_system(message: &Value) -> bool {
    message.get("role").and_then(Value::as_str) == Some("system")
}

/// True when the message carries `tool_result` blocks, which answer a `tool_use`
/// in the turn before it. Such a message can never start the tail.
fn carries_tool_result(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        })
}

fn split_index_by_token_fraction(messages: &[Value], fraction: f64) -> usize {
    if messages.is_empty() {
        return 0;
    }
    let fraction = fraction.clamp(0.05, 0.95);
    let total = messages.iter().map(message_tokens).sum::<u64>().max(1);
    let target = fraction * total as f64;
    let mut accumulated = 0u64;
    let mut index = messages.len().saturating_sub(1).max(1);
    for (position, message) in messages.iter().enumerate() {
        accumulated = accumulated.saturating_add(message_tokens(message));
        if accumulated as f64 >= target {
            index = (position + 1).max(1);
            break;
        }
    }
    // The tail must never be empty: pass 2 has nothing to rewrite against
    // without recent turns, and would just restate the note.
    index.min(messages.len().saturating_sub(1)).max(1)
}

/// Move the boundary forward past any message that would start the tail with an
/// orphaned `tool_result`.
fn snap_to_tool_boundary(messages: &[Value], mut index: usize) -> usize {
    while index < messages.len() && carries_tool_result(&messages[index]) {
        index += 1;
    }
    index.min(messages.len())
}

pub(super) fn split_for_two_pass(messages: &[Value], fraction: f64) -> Option<Split<'_>> {
    if messages.len() < MIN_MESSAGES_FOR_TWO_PASS {
        return None;
    }
    if messages.iter().map(message_tokens).sum::<u64>() < MIN_TOKENS_FOR_TWO_PASS {
        return None;
    }
    let index = snap_to_tool_boundary(messages, split_index_by_token_fraction(messages, fraction));
    if index == 0 || index >= messages.len() {
        return None;
    }
    Some(Split {
        prefix: &messages[..index],
        tail: &messages[index..],
    })
}

/// Bound the pass-1 note before it is carried into pass 2.
pub(super) fn note_for_pass2(pass1: &str) -> String {
    let note = pass1.trim();
    if note.chars().count() <= MAX_NOTE_CHARS {
        return note.to_string();
    }
    let mut bounded: String = note.chars().take(MAX_NOTE_CHARS).collect();
    bounded.push_str("\n\n[... earlier-history note truncated for the pass-2 input budget ...]");
    bounded
}

/// Pass-2 input: the system turns, the pass-1 note, the recent tail, and an
/// instruction that the note must be carried forward whole.
///
/// The "in full" wording is load-bearing. Without it a model with the recent
/// turns in front of it writes about those and refers back to the earlier note
/// rather than absorbing it, and the successor — which never sees the note —
/// loses the early history entirely.
pub(super) fn build_pass2_messages(split: &Split<'_>, note: &str) -> Vec<Value> {
    let mut messages: Vec<Value> = split
        .prefix
        .iter()
        .filter(|message| is_system(message))
        .cloned()
        .collect();

    messages.push(json!({
        "role": "user",
        "content": format!(
            "The earlier part of this conversation was summarised because it no \
             longer fits. That summary:\n\n<earlier_summary>\n{note}\n</earlier_summary>\n\n\
             The turns that follow are the most recent ones, which the summary does \
             not cover."
        ),
    }));
    messages.extend(split.tail.iter().cloned());
    messages.push(json!({
        "role": "user",
        "content": format!(
            "Write the final summary that a successor will rely on as its only \
             memory of this conversation. Incorporate the earlier summary in full — \
             do not omit sections, do not refer back to it, and do not drop early \
             history because newer turns are in front of you. Merge it with the \
             recent turns above into one self-contained account, keeping concrete \
             values, file paths, errors, decisions and outstanding work from \
             both.\n\n<earlier_summary>\n{note}\n</earlier_summary>"
        ),
    }));
    messages
}
