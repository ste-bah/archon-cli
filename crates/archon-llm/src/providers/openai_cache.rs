//! GPT-5.6 prompt-cache helpers for the Chat Completions body.
//!
//! Split from `openai.rs` for the 500-line gate.

/// Whether the model takes explicit `prompt_cache_breakpoint` markers.
///
/// GPT-5.6 introduced them; everything before it caches implicitly and rejects
/// `prompt_cache_options` outright, so this gate is what keeps the new fields
/// off requests that would 400 on them.
///
/// Parsed from the id rather than kept as a list, because a list of names goes
/// stale the moment OpenAI ships a variant — `gpt-5.6-mini`, a dated snapshot,
/// or a successor — and the failure mode of a stale list is silently losing
/// caching on a model that supports it. An operator whose gateway disagrees can
/// still force the answer with `prompt_cache_strategy`.
pub fn supports_explicit_prompt_cache(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    let Some(rest) = model.strip_prefix("gpt-") else {
        return false;
    };

    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = version.split('.');
    let Some(major) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);

    (major, minor) >= (5, 6)
}

/// Split the system prompt into content parts, closing the stable head with a
/// breakpoint.
///
/// The volatile tail — recalled memories, the inner voice, per-turn reminders —
/// deliberately lands in a second part *behind* the breakpoint, so it does not
/// invalidate the prefix it follows.
///
/// `stable_blocks` counts entries in `system`, while empty blocks are dropped
/// here, so the index can drift; it is clamped rather than trusted. Being off by
/// one costs a slightly shorter or longer cached prefix, never a malformed body.
pub(super) fn system_content_parts(
    system: &[serde_json::Value],
    stable_blocks: Option<usize>,
) -> Vec<serde_json::Value> {
    let texts: Vec<&str> = system
        .iter()
        .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
        .filter(|text| !text.is_empty())
        .collect();

    if texts.is_empty() {
        return Vec::new();
    }

    let at = stable_blocks
        .filter(|n| *n > 0 && *n < texts.len())
        .unwrap_or(texts.len());

    let mut parts = vec![serde_json::json!({
        "type": "text",
        "text": texts[..at].join("\n"),
        "prompt_cache_breakpoint": crate::cache_wire::breakpoint_marker(),
    })];

    let tail = texts[at..].join("\n");
    if !tail.is_empty() {
        parts.push(serde_json::json!({ "type": "text", "text": tail }));
    }

    parts
}
