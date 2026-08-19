//! Reclaiming context without asking a model (#189 Phase 8).
//!
//! Compaction always calls a model — `generate_compaction_summary_with_usage`
//! and its segment twin. But much of what fills a window is removable by
//! arithmetic: a file read three times with nothing writing to it in between,
//! an error retried successfully a moment later, a tool result whose complete
//! text is already on disk from Phase 1. Summarising those spends a request and
//! a wait to reach a conclusion this pass reaches for free.
//!
//! # Every rule rewrites, none deletes
//!
//! An assistant `tool_use` block whose `tool_result` is missing is a
//! provider-level error, not a degradation — `fill_missing_tool_results` exists
//! because that failure is real. So no rule here removes a block. Each replaces
//! the *content string* of a `tool_result` with a shorter one that says where
//! the original went, which makes orphaning structurally impossible rather than
//! something the rules have to remember not to do.
//!
//! That also constrains what "drop the retried error" can mean: the block stays
//! and its body becomes a one-line note. The tokens go; the pairing does not.

use std::collections::HashMap;

use crate::config::PruneConfig;

/// What a pruning pass achieved.
#[derive(Debug, Clone, PartialEq)]
pub struct PruneOutcome {
    /// The rewritten history. Same length, same block structure.
    pub messages: Vec<serde_json::Value>,
    /// Bytes removed from tool-result content.
    pub bytes_reclaimed: usize,
    /// Which rules fired, for telemetry and for explaining a skipped summary.
    pub rules_fired: Vec<&'static str>,
}

impl PruneOutcome {
    fn unchanged(messages: &[serde_json::Value]) -> Self {
        Self {
            messages: messages.to_vec(),
            bytes_reclaimed: 0,
            rules_fired: Vec::new(),
        }
    }

    /// Whether anything was actually reclaimed.
    #[must_use]
    pub fn reclaimed_anything(&self) -> bool {
        self.bytes_reclaimed > 0
    }
}

pub(crate) const RULE_SPILLED: &str = "spilled_superseded";
pub(crate) const RULE_REPEATED_READS: &str = "repeated_reads";
pub(crate) const RULE_RETRIED_ERRORS: &str = "retried_errors";

/// Rewrite what can be reclaimed mechanically.
///
/// Runs before the model summarisation path. Rules are applied to every
/// eligible result regardless of size: a rule that only fires on large messages
/// leaves free bytes on the floor for no reason.
pub fn prune_mechanical(messages: &[serde_json::Value], config: PruneConfig) -> PruneOutcome {
    if !config.enabled || !config.any_rule_enabled() {
        return PruneOutcome::unchanged(messages);
    }

    let calls = ToolCalls::index(messages);
    let mut replacements: HashMap<String, (String, &'static str)> = HashMap::new();

    if config.spilled_superseded {
        plan_spilled_supersessions(messages, &calls, &mut replacements);
    }
    if config.repeated_reads {
        plan_repeated_reads(&calls, &mut replacements);
    }
    if config.retried_errors {
        plan_retried_errors(messages, &calls, &mut replacements);
    }
    if replacements.is_empty() {
        return PruneOutcome::unchanged(messages);
    }
    apply(messages, &replacements)
}

/// `tool_use` blocks indexed by id, in emission order.
struct ToolCalls {
    /// id -> (ordinal, name, canonical input)
    by_id: HashMap<String, (usize, String, String)>,
}

impl ToolCalls {
    fn index(messages: &[serde_json::Value]) -> Self {
        let mut by_id = HashMap::new();
        let mut ordinal = 0;
        for block in blocks(messages) {
            if block.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
                continue;
            }
            let (Some(id), Some(name)) = (
                block.get("id").and_then(|v| v.as_str()),
                block.get("name").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let input = block
                .get("input")
                .map(|value| value.to_string())
                .unwrap_or_default();
            by_id.insert(id.to_string(), (ordinal, name.to_string(), input));
            ordinal += 1;
        }
        Self { by_id }
    }

    fn get(&self, id: &str) -> Option<&(usize, String, String)> {
        self.by_id.get(id)
    }
}

fn blocks(messages: &[serde_json::Value]) -> impl Iterator<Item = &serde_json::Value> {
    messages
        .iter()
        .filter_map(|message| message.get("content").and_then(|v| v.as_array()))
        .flatten()
}

/// Every `tool_result` as (id, ordinal of its call, is_error, content).
fn results(
    messages: &[serde_json::Value],
    calls: &ToolCalls,
) -> Vec<(String, usize, bool, String)> {
    let mut found = Vec::new();
    for block in blocks(messages) {
        if block.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
            continue;
        }
        let (Some(id), Some(content)) = (
            block.get("tool_use_id").and_then(|v| v.as_str()),
            block.get("content").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let Some((ordinal, _, _)) = calls.get(id) else {
            continue;
        };
        let is_error = block
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        found.push((id.to_string(), *ordinal, is_error, content.to_string()));
    }
    found
}

/// A spilled result whose text is repeated verbatim later.
///
/// The later copy is the one the model will reach for, and the earlier body is
/// on disk, so the earlier one can become a pointer. Only spilled results
/// qualify: without a locator there would be nowhere to point.
fn plan_spilled_supersessions(
    messages: &[serde_json::Value],
    calls: &ToolCalls,
    out: &mut HashMap<String, (String, &'static str)>,
) {
    let spills = spill_paths(messages);
    let all = results(messages, calls);
    for (index, (id, _, _, content)) in all.iter().enumerate() {
        let Some(path) = spills.get(id) else {
            continue;
        };
        let superseded = all
            .iter()
            .skip(index + 1)
            .any(|(_, _, _, later)| later == content);
        if !superseded {
            continue;
        }
        out.insert(
            id.clone(),
            (
                format!(
                    "[Archon pruned: this result is repeated later in the conversation. \
                     Full output: {path} (read it if you need this copy).]"
                ),
                RULE_SPILLED,
            ),
        );
    }
}

fn spill_paths(messages: &[serde_json::Value]) -> HashMap<String, String> {
    blocks(messages)
        .filter_map(|block| {
            let id = block.get("tool_use_id").and_then(|v| v.as_str())?;
            let path = block.get(super::SPILL_PATH_KEY).and_then(|v| v.as_str())?;
            Some((id.to_string(), path.to_string()))
        })
        .collect()
}

/// Repeated reads of a path nothing wrote to in between.
///
/// The intervening-write check is what keeps this sound: collapsing two reads
/// across an `Edit` would present stale content as current, which is worse than
/// spending the tokens.
fn plan_repeated_reads(calls: &ToolCalls, out: &mut HashMap<String, (String, &'static str)>) {
    let mut reads: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    let mut writes: HashMap<String, Vec<usize>> = HashMap::new();
    for (id, (ordinal, name, input)) in &calls.by_id {
        let Some(path) = path_argument(input) else {
            continue;
        };
        if name == "Read" || name == "NotebookRead" {
            reads.entry(path).or_default().push((*ordinal, id.clone()));
        } else if matches!(name.as_str(), "Edit" | "Write" | "NotebookEdit") {
            writes.entry(path).or_default().push(*ordinal);
        }
    }

    for (path, mut same_path) in reads {
        if same_path.len() < 2 {
            continue;
        }
        same_path.sort_by_key(|(ordinal, _)| *ordinal);
        let written = writes.get(&path).cloned().unwrap_or_default();
        let (last_ordinal, _) = same_path[same_path.len() - 1];
        for (ordinal, id) in same_path.iter().take(same_path.len() - 1) {
            // Any write between this read and the final one makes them
            // different content; collapsing would assert otherwise.
            if written
                .iter()
                .any(|write| write > ordinal && *write < last_ordinal)
            {
                continue;
            }
            out.insert(
                id.clone(),
                (
                    format!(
                        "[Archon pruned: {path} is read again later with no intervening edit; \
                         see the later result for its contents.]"
                    ),
                    RULE_REPEATED_READS,
                ),
            );
        }
    }
}

/// A failed call that the same call, with the same input, later survived.
fn plan_retried_errors(
    messages: &[serde_json::Value],
    calls: &ToolCalls,
    out: &mut HashMap<String, (String, &'static str)>,
) {
    let all = results(messages, calls);
    for (id, ordinal, is_error, _) in &all {
        if !is_error {
            continue;
        }
        let Some((_, name, input)) = calls.get(id) else {
            continue;
        };
        let retried = all.iter().any(|(other_id, other_ordinal, other_error, _)| {
            if *other_error || other_ordinal <= ordinal {
                return false;
            }
            calls
                .get(other_id)
                .is_some_and(|(_, other_name, other_input)| {
                    other_name == name && other_input == input
                })
        });
        if retried {
            out.insert(
                id.clone(),
                (
                    format!("[Archon pruned: this {name} call failed and the same call succeeded later.]"),
                    RULE_RETRIED_ERRORS,
                ),
            );
        }
    }
}

/// The path a tool was pointed at, if its input names one.
fn path_argument(input_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(input_json).ok()?;
    for key in ["file_path", "path", "notebook_path"] {
        if let Some(path) = value.get(key).and_then(|v| v.as_str()) {
            return Some(path.to_string());
        }
    }
    None
}

/// Rewrite the planned results, leaving every block in place.
fn apply(
    messages: &[serde_json::Value],
    replacements: &HashMap<String, (String, &'static str)>,
) -> PruneOutcome {
    let mut bytes_reclaimed = 0usize;
    let mut rules_fired: Vec<&'static str> = Vec::new();
    let rewritten = messages
        .iter()
        .map(|message| {
            let Some(blocks) = message.get("content").and_then(|v| v.as_array()) else {
                return message.clone();
            };
            let mut touched = false;
            let projected: Vec<serde_json::Value> = blocks
                .iter()
                .map(|block| {
                    let Some((body, rule)) = block
                        .get("tool_use_id")
                        .and_then(|v| v.as_str())
                        .and_then(|id| replacements.get(id))
                    else {
                        return block.clone();
                    };
                    let Some(existing) = block.get("content").and_then(|v| v.as_str()) else {
                        return block.clone();
                    };
                    // A rule that made a result longer would be a bug reported
                    // as a saving; leave it alone instead.
                    if body.len() >= existing.len() {
                        return block.clone();
                    }
                    touched = true;
                    bytes_reclaimed += existing.len() - body.len();
                    if !rules_fired.contains(rule) {
                        rules_fired.push(rule);
                    }
                    replace_content(block, body)
                })
                .collect();
            if !touched {
                return message.clone();
            }
            replace_array(message, projected)
        })
        .collect();
    rules_fired.sort_unstable();
    PruneOutcome {
        messages: rewritten,
        bytes_reclaimed,
        rules_fired,
    }
}

/// Rebuild an object with a new `content`, preserving key order.
///
/// Order matters: cache markers are position-sensitive, so a rebuilt block has
/// to serialize exactly as an in-place overwrite would.
fn replace_content(block: &serde_json::Value, body: &str) -> serde_json::Value {
    rebuild(block, serde_json::Value::String(body.to_string()))
}

fn replace_array(message: &serde_json::Value, blocks: Vec<serde_json::Value>) -> serde_json::Value {
    rebuild(message, serde_json::Value::Array(blocks))
}

fn rebuild(value: &serde_json::Value, content: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut content = Some(content);
    serde_json::Value::Object(
        object
            .iter()
            .map(|(key, existing)| {
                let entry = (key == "content")
                    .then(|| content.take())
                    .flatten()
                    .unwrap_or_else(|| existing.clone());
                (key.clone(), entry)
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "prune_tests.rs"]
mod tests;
