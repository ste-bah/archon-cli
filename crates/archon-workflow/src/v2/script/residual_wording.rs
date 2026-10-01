//! Batch O2 (REM-17): a residual round's claim carries every gap's text and
//! every recording verifier's summary WHOLE; nothing is sliced to fit.
//!
//! Earlier binaries cut a gap's description to 800 characters, a recording
//! summary to 600 and a refused review verdict to 4 000 (six blocker
//! excerpts of 600). A claim is a dispatched call input a resumed run
//! replays, and the host's dispatch check rebuilds it, so a round a call was
//! ALREADY dispatched under with that cut text keeps it ([`worded`]): its
//! recorded answer replays and its check passes. Every other round -- one
//! never dispatched, or dispatched under the whole text -- gets the whole
//! text.

use serde_json::Value;

use super::super::{WorkflowV2CallRecord, WorkflowV2HostMethod};

const LEGACY_DESCRIPTION_CHARS: usize = 800;
const LEGACY_SUMMARY_CHARS: usize = 600;
const LEGACY_REFUSAL_CHARS: usize = 4_000;
const LEGACY_EVIDENCE: usize = 6;

/// Which text a claim is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Wording {
    /// Every text whole (every round not dispatched under the cut text).
    Whole,
    /// The cut text earlier binaries dispatched, for their recorded rounds.
    Legacy,
}

impl Wording {
    pub(super) fn description(self, text: &str) -> String {
        self.cut(text, LEGACY_DESCRIPTION_CHARS)
    }

    pub(super) fn summary(self, text: &str) -> String {
        self.cut(text, LEGACY_SUMMARY_CHARS)
    }

    /// A refused review verdict as a review round carries it: whole, or as
    /// earlier binaries cut it (only that shape was ever cut).
    pub(super) fn refusal(self, refusal: &Value) -> Value {
        let mut out = refusal.clone();
        if self == Self::Whole || refusal.get("review_prompt").is_none() {
            return out;
        }
        for field in ["summary", "review_prompt"] {
            if let Some(text) = refusal.get(field).and_then(Value::as_str) {
                out[field] = Value::String(self.cut(text, LEGACY_REFUSAL_CHARS));
            }
        }
        if let Some(evidence) = refusal.get("blocker_evidence").and_then(Value::as_array) {
            let cut: Vec<Value> = evidence
                .iter()
                .take(LEGACY_EVIDENCE)
                .map(|entry| {
                    let mut entry = entry.clone();
                    if let Some(text) = entry.get("summary").and_then(Value::as_str) {
                        entry["summary"] = Value::String(self.summary(text));
                    }
                    entry
                })
                .collect();
            out["blocker_evidence"] = Value::Array(cut);
        }
        out
    }

    fn cut(self, text: &str, limit: usize) -> String {
        if self == Self::Whole || text.chars().count() <= limit {
            return text.to_string();
        }
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}

/// The claim for round `key`: `build(Legacy)` when a call recorded for the
/// round was dispatched carrying exactly that cut text, else
/// `build(Whole)`.
pub(super) fn worded(
    stored: &[WorkflowV2CallRecord],
    key: &str,
    build: impl Fn(Wording) -> String,
) -> String {
    let whole = build(Wording::Whole);
    let legacy = build(Wording::Legacy);
    if legacy != whole && dispatched_under(stored, key, &legacy) {
        legacy
    } else {
        whole
    }
}

/// Whether a call recorded for round `key` (never a checkpoint) carries
/// `text` in its prompt: verbatim, or JSON-quoted once or twice (the
/// prelude quotes a claim inside a finding, and a finding inside a prompt).
fn dispatched_under(stored: &[WorkflowV2CallRecord], key: &str, text: &str) -> bool {
    let once = quoted(text);
    let twice = quoted(&once);
    stored.iter().any(|record| {
        record.call.method != WorkflowV2HostMethod::Checkpoint
            && super::second_pass::residual_key(&record.call) == Some(key)
            && record.call.options.task.as_deref().is_some_and(|prompt| {
                prompt.contains(text) || prompt.contains(&once) || prompt.contains(&twice)
            })
    })
}

/// Whether `prompt` carries `text` whole: verbatim, or JSON-quoted up to
/// three times (a gap inside a claim, a claim inside a finding, a finding
/// inside a prompt).
pub(super) fn carries(prompt: &str, text: &str) -> bool {
    let mut form = text.to_string();
    for _ in 0..4 {
        if prompt.contains(&form) {
            return true;
        }
        form = quoted(&form);
    }
    false
}

/// Whether a call recorded for round `key` was dispatched carrying the cut
/// form of `description` an earlier binary built, and not the whole of it.
pub(super) fn dispatched_cut(
    stored: &[WorkflowV2CallRecord],
    key: &str,
    description: &str,
) -> bool {
    let cut = Wording::Legacy.description(description);
    let Some(cut) = cut.strip_suffix("...").filter(|_| cut != description) else {
        return false;
    };
    stored.iter().any(|record| {
        record.call.method != WorkflowV2HostMethod::Checkpoint
            && super::second_pass::residual_key(&record.call) == Some(key)
            && record
                .call
                .options
                .task
                .as_deref()
                .is_some_and(|prompt| carries(prompt, cut) && !carries(prompt, description))
    })
}

fn quoted(text: &str) -> String {
    let json = serde_json::to_string(text).unwrap_or_default();
    json.get(1..json.len().saturating_sub(1))
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
#[path = "residual_wording_tests.rs"]
mod tests;
