//! Turning a request into a cassette key (#189 Phase 5).
//!
//! The whole scheme rests on this: a request recorded on Tuesday has to hash to
//! the same key when it is replayed on Friday. Everything that varies between
//! two otherwise identical runs has to come out first, and everything that
//! changes the *answer* has to stay in. Too little normalisation and nothing
//! ever hits; too much and two different requests share a cassette, which is
//! worse — a test would pass against the wrong recording.

use sha2::{Digest, Sha256};

use crate::provider::LlmRequest;

/// Keys stripped wherever they appear in a request.
///
/// Each varies run to run without changing what the model is being asked:
///
/// - `id`, `tool_use_id`, `tool_call_id` — freshly generated per turn. The
///   *sequence* of tool calls survives, which is what distinguishes one
///   conversation from another; only the labels go.
/// - `cache_control` — prompt-cache breakpoints move as the context grows and
///   are instructions to the provider's cache, not part of the question.
/// - `archon_spill` — the spill locator #189 Phase 1 attaches to oversized tool
///   results. Already stripped on the way to the wire; stripped here too, so a
///   cassette cannot depend on a path under someone's home directory.
/// - `signature` — the provider's own attestation over a thinking block.
/// - `timestamp`, `created_at`, `request_id` — self-evidently per-run.
/// - `run_id`, `session_id` — fresh UUIDs in the `archon_runtime` envelope
///   carried under `extra`. Found by recording the same subagent run twice and
///   diffing the two canonical forms: these were the *only* fields that
///   differed, and without them nothing ever hit. Its `turn` and `round` fields
///   are left alone on purpose — they are what tells turn one from turn two.
const VOLATILE_KEYS: &[&str] = &[
    "id",
    "tool_use_id",
    "tool_call_id",
    "cache_control",
    "archon_spill",
    "signature",
    "timestamp",
    "created_at",
    "request_id",
    "run_id",
    "session_id",
];

/// Stable key for `request`.
///
/// Hex rather than raw bytes because this becomes a filename, and truncated to
/// 32 characters because the full 64 makes directory listings unreadable while
/// 128 bits is far past any collision risk for a directory of cassettes.
pub fn digest(request: &LlmRequest) -> String {
    let canonical = canonical_json(request);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    hex::encode(hasher.finalize())[..32].to_string()
}

/// The exact bytes that get hashed.
///
/// Public within the crate so a cassette can carry them: when a replay misses,
/// the only useful question is "how does this request differ from the one that
/// was recorded", and that is unanswerable without both canonical forms.
pub(crate) fn canonical_json(request: &LlmRequest) -> String {
    let mut fields: Vec<(&str, serde_json::Value)> = vec![
        ("model", serde_json::json!(request.model)),
        ("max_tokens", serde_json::json!(request.max_tokens)),
        ("system", serde_json::Value::Array(request.system.clone())),
        (
            "messages",
            serde_json::Value::Array(request.messages.clone()),
        ),
        (
            "tools",
            serde_json::Value::Array(request.tools.as_ref().clone()),
        ),
        ("thinking", json_or_null(request.thinking.clone())),
        ("speed", json_or_null(request.speed.clone())),
        ("effort", json_or_null(request.effort.clone())),
        ("extra", request.extra.clone()),
    ];
    // `request_origin` and `reasoning_encrypted` are deliberately absent.
    // The first is a tracing marker ("main_session" / "subagent") that no
    // provider reads; the second is an opaque blob the provider hands back and
    // that differs on every turn even when the conversation does not.
    fields.sort_by_key(|(name, _)| *name);

    let mut canonical = serde_json::Map::new();
    for (name, value) in fields {
        canonical.insert(name.to_string(), strip_volatile(value));
    }
    // `to_string` over a sorted structure: `serde_json::Map` is a `BTreeMap`
    // unless `preserve_order` is on, and sorting explicitly means this holds
    // either way rather than depending on a feature flag somebody else sets.
    serde_json::to_string(&sorted(serde_json::Value::Object(canonical)))
        .unwrap_or_else(|_| String::from("{}"))
}

fn json_or_null<T: serde::Serialize>(value: Option<T>) -> serde_json::Value {
    value.map_or(serde_json::Value::Null, |inner| {
        serde_json::to_value(inner).unwrap_or(serde_json::Value::Null)
    })
}

/// Remove [`VOLATILE_KEYS`] everywhere in the tree.
fn strip_volatile(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(key, _)| !VOLATILE_KEYS.contains(&key.as_str()))
                .map(|(key, inner)| (key, strip_volatile(inner)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(strip_volatile).collect())
        }
        other => other,
    }
}

/// Rebuild every object with its keys in sorted order.
fn sorted(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> = map
                .into_iter()
                .map(|(key, inner)| (key, sorted(inner)))
                .collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            serde_json::Value::Object(entries.into_iter().collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sorted).collect())
        }
        other => other,
    }
}

#[cfg(test)]
#[path = "replay_digest_tests.rs"]
mod tests;
