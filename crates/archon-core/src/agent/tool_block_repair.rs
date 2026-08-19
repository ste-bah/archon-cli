//! Repair `tool_use` content blocks that a proxy split in two.
//!
//! # The malformation
//!
//! LiteLLM's Anthropic adapter opens a NEW content block in the middle of a
//! tool call's arguments when a model emits parallel tool calls, so one logical
//! call arrives as a named block plus an unnamed continuation. Captured from a
//! live sglang deployment:
//!
//! ```text
//! START idx=2 tool_use name=Read     {"file_path": "/tmp/a.md"}   (valid)
//! START idx=4 tool_use name=Read     {"file_path": "/tmp/b.md"    (truncated)
//! START idx=6 tool_use name=""       }                            (the orphan)
//! START idx=7 tool_use name=Grep     {"pattern": "foo", ...}      (valid)
//! ```
//!
//! Archon then reports, correctly:
//!
//! ```text
//! Tool 'Read' produced malformed JSON input ... EOF while parsing
//! Tool ''     produced malformed JSON input ... expected value at column 1
//! ```
//!
//! A `tool_use` `content_block_start` always carries `id` and `name` in the
//! Anthropic streaming spec, and the accumulation contract is one string per
//! block index parsed at `content_block_stop`. An unnamed `tool_use` block is
//! therefore not a shape any conforming producer emits — which is what makes
//! repairing it safe rather than a guess.
//!
//! # Why this cannot corrupt a good call
//!
//! Three independent gates, any one of which would be sufficient:
//!
//! 1. `[api] repair_split_tool_blocks` — the operator can switch it off.
//! 2. It only runs at all when an unnamed `tool_use` block is present, which is
//!    already-invalid input. A conforming stream never reaches this code.
//! 3. A merge is committed ONLY if the result parses, and only if EXACTLY ONE
//!    candidate accepts the fragment. A block whose JSON already parses is
//!    never a candidate, so a healthy call cannot be a merge target, and an
//!    ambiguous fragment is refused rather than attached to a coin-flip winner.
//!
//! When it refuses, the caller falls through to the existing error. Failing
//! loudly is the correct outcome — Anthropic's own SDKs keep an empty input and
//! let `stop_reason` explain rather than reconstruct a call they cannot verify.

/// One accepted merge: append `orphan`'s accumulated JSON to `target`'s, then
/// drop `orphan`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SplitToolRepair {
    pub target: usize,
    pub orphan: usize,
}

fn parses(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json.trim()).is_ok()
}

/// Plan the repairs for one message's pending tool calls.
///
/// `names` and `jsons` are parallel and in stream order. Returns the merges to
/// apply, in application order; an empty result means "nothing to repair, or
/// nothing that could be verified".
pub(crate) fn plan_split_tool_repairs(names: &[&str], jsons: &[&str]) -> Vec<SplitToolRepair> {
    debug_assert_eq!(names.len(), jsons.len());
    if !names.iter().any(|name| name.trim().is_empty()) {
        return Vec::new();
    }
    // Work on a mutable copy so each accepted merge changes what the next
    // orphan sees. Repairing one truncated call can make it parse, which
    // removes it as a candidate for the following fragment — that is what keeps
    // two truncated calls in the same message unambiguous.
    let mut working: Vec<String> = jsons.iter().map(|json| (*json).to_string()).collect();
    let mut merged: Vec<bool> = vec![false; names.len()];
    let mut plan = Vec::new();

    for orphan in 0..names.len() {
        if !names[orphan].trim().is_empty() || merged[orphan] {
            continue;
        }
        let fragment = working[orphan].clone();
        if fragment.trim().is_empty() {
            continue;
        }
        let accepted: Vec<usize> = (0..names.len())
            .filter(|candidate| {
                *candidate != orphan
                    && !merged[*candidate]
                    && !names[*candidate].trim().is_empty()
                    && !parses(&working[*candidate])
                    && parses(&format!("{}{}", working[*candidate], fragment))
            })
            .collect();
        if accepted.len() != 1 {
            continue;
        }
        let target = accepted[0];
        working[target] = format!("{}{}", working[target], fragment);
        merged[orphan] = true;
        plan.push(SplitToolRepair { target, orphan });
    }
    plan
}

/// Apply the planned merges to parallel `(name, id, input_json)` accessors.
///
/// Returns the orphan positions to drop, highest first so the caller can remove
/// them without shifting the ones it has not removed yet.
pub(crate) fn apply_split_tool_repairs(
    jsons: &mut [String],
    plan: &[SplitToolRepair],
    names: &[&str],
) -> Vec<usize> {
    let mut drop_positions = Vec::with_capacity(plan.len());
    for repair in plan {
        let fragment = jsons[repair.orphan].clone();
        jsons[repair.target].push_str(&fragment);
        drop_positions.push(repair.orphan);
        tracing::warn!(
            tool = %names.get(repair.target).copied().unwrap_or_default(),
            orphan_fragment_len = fragment.len(),
            "repaired a tool_use block the provider split across two content \
             blocks; the stream was not valid Anthropic protocol. Set \
             [api] repair_split_tool_blocks = false to disable this repair."
        );
    }
    drop_positions.sort_unstable();
    drop_positions.reverse();
    drop_positions
}

/// Repair the pending tool calls of one assistant turn in place.
///
/// The single entry point for both agent paths: it reads the `[api]` flag,
/// plans, verifies and applies. A conforming stream returns immediately without
/// allocating, so this is free on every provider that does not split blocks.
pub(crate) fn repair_pending_tool_calls(pending: &mut Vec<super::PendingToolCall>) {
    // Read at use time rather than held on `AgentConfig`, which is constructed
    // in too many places to thread a new field through for a decision taken
    // once per turn. Same approach as prune and spill.
    let enabled = crate::config::load_config()
        .map(|loaded| loaded.api.repair_split_tool_blocks)
        .unwrap_or(true);
    if !enabled {
        return;
    }
    let names: Vec<&str> = pending.iter().map(|tool| tool.name.as_str()).collect();
    let jsons: Vec<&str> = pending.iter().map(|tool| tool.input_json.as_str()).collect();
    let plan = plan_split_tool_repairs(&names, &jsons);
    if plan.is_empty() {
        return;
    }
    let mut jsons: Vec<String> = pending.iter().map(|tool| tool.input_json.clone()).collect();
    let drop_positions = apply_split_tool_repairs(&mut jsons, &plan, &names);
    for (tool, json) in pending.iter_mut().zip(jsons) {
        tool.input_json = json;
    }
    for position in drop_positions {
        pending.remove(position);
    }
}

#[cfg(test)]
#[path = "tool_block_repair_tests.rs"]
mod tests;
