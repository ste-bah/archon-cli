use std::collections::HashSet;
use std::sync::{Mutex as StdMutex, OnceLock};

use crate::effort::EFFORT_BETA;
use crate::fast_mode::FAST_MODE_BETA;

#[derive(Debug, Clone)]
pub struct MessageRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: Vec<serde_json::Value>,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub thinking: Option<serde_json::Value>,
    /// When fast mode is active, set to `Some("fast")`.
    pub speed: Option<String>,
    /// When effort is not High, set to the effort level string (e.g. `"low"`, `"medium"`).
    pub effort: Option<String>,
    /// Diagnostic marker: None, "main_session", or "subagent".
    pub request_origin: Option<String>,
}

impl Default for MessageRequest {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8192,
            system: Vec::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            thinking: None,
            speed: None,
            effort: None,
            request_origin: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("authentication error: {0}")]
    AuthError(String),

    #[error("rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("server overloaded (529)")]
    Overloaded,

    #[error("server error ({status}): {message}")]
    ServerError { status: u16, message: String },

    #[error("serialization error: {0}")]
    SerializeError(String),
}

/// Wire name of the `speed` knob, used as its key in the unsupported registry
/// and in the dropped-knob warning.
pub(crate) const SPEED_KNOB: &str = "speed";
/// Wire name of the effort knob.
pub(crate) const EFFORT_KNOB: &str = "output_config.effort";

/// #123: `speed` is sent for any model unless the API has rejected it.
///
/// This replaced `supports_speed`, a stub that returned `false` for every
/// model — so fast mode was silently dropped on the wire for all of them while
/// `/fast` still reported success. Same failure as the effort allowlist, same
/// fix.
pub(crate) fn effective_speed(request: &MessageRequest) -> Option<&str> {
    let value = request.speed.as_deref()?;
    if knob_known_unsupported(&request.model, SPEED_KNOB) {
        warn_dropped_knob(&request.model, SPEED_KNOB, value);
        return None;
    }
    Some(value)
}

/// Project the canonical effort ladder onto Anthropic's `output_config.effort`.
///
/// `high` and `max` both map to `None`: omitting the field already means high
/// on this API, and Anthropic has no rung above it — `max` is expressed through
/// the `thinking` parameter instead (see `select_thinking_mode`). The resulting
/// wire bytes are identical to the pre-#123 behaviour, when the core layer
/// omitted those levels before the request was ever built.
///
/// **This is where the omission decision lives, and it must not move back up
/// into the shared layer.** On an OpenAI-compatible backend an absent
/// `reasoning_effort` means "no reasoning at all", not "high", so a shared
/// `High => None` silently disables reasoning on vLLM. The core now always
/// sends a concrete level and each provider clamps it here.
pub(crate) fn effective_effort(request: &MessageRequest) -> Option<&str> {
    let value = request.effort.as_deref()?;
    // Checked first: these levels are a no-op on every Anthropic model, so
    // warning that they were "dropped" would be noise.
    if matches!(value, "high" | "max") {
        return None;
    }
    // Model-agnostic: the knob is sent for ANY model unless this process has
    // actually watched the API reject it. See `mark_knob_unsupported`.
    if knob_known_unsupported(&request.model, EFFORT_KNOB) {
        warn_dropped_knob(&request.model, EFFORT_KNOB, value);
        return None;
    }
    Some(value)
}

/// Append the betas required by whichever conditional knobs this request
/// actually carries, preserving any the identity already set.
///
/// The betas are NOT sent unconditionally: the API rejects betas for features
/// that are not active, so each one is gated on its knob surviving
/// `effective_speed` / `effective_effort`. That keeps the header and the body
/// in lockstep — including after the degrade path switches a knob off.
pub(crate) fn apply_conditional_betas(
    request: &MessageRequest,
    headers: &mut std::collections::HashMap<String, String>,
) {
    let mut extra: Vec<&str> = Vec::new();
    if effective_speed(request).is_some() {
        extra.push(FAST_MODE_BETA);
    }
    if effective_effort(request).is_some() {
        extra.push(EFFORT_BETA);
    }
    if extra.is_empty() {
        return;
    }
    let existing = headers.get("anthropic-beta").cloned().unwrap_or_default();
    let combined = if existing.is_empty() {
        extra.join(",")
    } else {
        format!("{},{}", existing, extra.join(","))
    };
    headers.insert("anthropic-beta".into(), combined);
}

/// `(model, knob)` pairs this process has observed the API reject.
///
/// #123 replaced two hardcoded gates with runtime learning: a model allowlist
/// for effort (wrong in both directions — it dropped the knob on `opus-5` and
/// `sonnet-4-6`, and needed a code edit per release) and a blanket `false`
/// stub for speed. Both knobs are now sent by default and switched off per
/// model only when the API says no.
static UNSUPPORTED_KNOBS: OnceLock<StdMutex<HashSet<(String, &'static str)>>> = OnceLock::new();

fn unsupported_knobs() -> &'static StdMutex<HashSet<(String, &'static str)>> {
    UNSUPPORTED_KNOBS.get_or_init(|| StdMutex::new(HashSet::new()))
}

pub(crate) fn knob_known_unsupported(model: &str, knob: &'static str) -> bool {
    unsupported_knobs()
        .lock()
        .map(|set| set.contains(&(model.to_string(), knob)))
        .unwrap_or(false)
}

/// Record that `model` rejects `knob`. Returns `true` if this is the first
/// time, i.e. the caller should rebuild and retry the request once. Returns
/// `false` on a repeat so a persistent 400 cannot loop.
pub(crate) fn mark_knob_unsupported(model: &str, knob: &'static str) -> bool {
    unsupported_knobs()
        .lock()
        .map(|mut set| set.insert((model.to_string(), knob)))
        .unwrap_or(false)
}

/// Whether this error response means "retry without the knob it blames".
///
/// Records the `(model, knob)` pair as a side effect so the rebuilt request
/// omits it and every later request for that model skips it too. Returns
/// `false` on a repeat — `mark_knob_unsupported` only reports the first
/// sighting — so a 400 that merely mentions a knob for some unrelated reason
/// cannot put the caller into a retry loop.
pub(crate) fn should_retry_without_knob(request: &MessageRequest, status: u16, body: &str) -> bool {
    if status != 400 {
        return false;
    }
    let Some(knob) = rejected_knob(request, body) else {
        return false;
    };
    if !mark_knob_unsupported(&request.model, knob) {
        return false;
    }
    tracing::warn!(
        model = %request.model,
        knob,
        "Anthropic rejected a request knob; retrying without it and disabling it for this model"
    );
    true
}

/// Which knob a 400 body blames, if any — and only among knobs the request
/// actually carried.
///
/// Deliberately narrow: a blanket "any 400 disables the knobs" would switch
/// them off on the first context-length or malformed-request error and never
/// switch them back on.
pub(crate) fn rejected_knob(request: &MessageRequest, body: &str) -> Option<&'static str> {
    let unknown_beta = extract_unknown_beta(body);
    if effective_effort(request).is_some()
        && (unknown_beta.as_deref() == Some(EFFORT_BETA) || body.contains("output_config"))
    {
        return Some(EFFORT_KNOB);
    }
    if effective_speed(request).is_some()
        && (unknown_beta.as_deref() == Some(FAST_MODE_BETA) || body.contains("\"speed\""))
    {
        return Some(SPEED_KNOB);
    }
    None
}

// #123: `supports_output_effort` is gone. It was a hand-maintained substring
// allowlist (`opus-4` / `sonnet-5` / `fable-5`) that silently dropped the knob
// on every model not named in it — `opus-5` and `sonnet-4-6` included — and
// needed a code edit per release. Replacing it with a wider substring match
// would have kept the same failure mode, just further out. The knob is now
// model-agnostic and self-correcting: see `effort_known_unsupported`.

fn warn_dropped_knob(model: &str, field: &str, value: &str) {
    static WARNED: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();
    let key = format!("{model}:{field}");
    let warned = WARNED.get_or_init(|| StdMutex::new(HashSet::new()));
    let Ok(mut guard) = warned.lock() else {
        return;
    };
    if guard.insert(key) {
        tracing::warn!(
            provider = "anthropic",
            model,
            field,
            value,
            "dropping unsupported Anthropic request knob"
        );
    }
}

const MAX_ANTHROPIC_CACHE_BREAKPOINTS: usize = 4;

pub(crate) fn remove_cache_directives(body: &mut serde_json::Value) {
    for path in cache_directive_paths(body) {
        remove_cache_directive(body, path);
    }
}

pub(crate) fn enforce_cache_breakpoint_budget(body: &mut serde_json::Value) {
    let mut directive_paths = cache_directive_paths(body);
    if directive_paths.len() <= MAX_ANTHROPIC_CACHE_BREAKPOINTS {
        return;
    }

    let excess = directive_paths.len() - MAX_ANTHROPIC_CACHE_BREAKPOINTS;
    for path in directive_paths.drain(..excess) {
        remove_cache_directive(body, path);
    }
}

#[derive(Clone, Copy)]
enum CacheDirectivePath {
    Tool(usize),
    System(usize),
    MessageBlock(usize, usize),
}

fn cache_directive_paths(body: &serde_json::Value) -> Vec<CacheDirectivePath> {
    let mut paths = Vec::new();
    collect_array_directives(body.get("tools"), CacheDirectivePath::Tool, &mut paths);
    collect_array_directives(body.get("system"), CacheDirectivePath::System, &mut paths);
    if let Some(messages) = body.get("messages").and_then(serde_json::Value::as_array) {
        for (message_index, message) in messages.iter().enumerate() {
            let Some(blocks) = message.get("content").and_then(serde_json::Value::as_array) else {
                continue;
            };
            for (block_index, block) in blocks.iter().enumerate() {
                if block.get("cache_control").is_some() {
                    paths.push(CacheDirectivePath::MessageBlock(message_index, block_index));
                }
            }
        }
    }
    paths
}

fn collect_array_directives(
    value: Option<&serde_json::Value>,
    path: impl Fn(usize) -> CacheDirectivePath,
    paths: &mut Vec<CacheDirectivePath>,
) {
    let Some(values) = value.and_then(serde_json::Value::as_array) else {
        return;
    };
    for (index, value) in values.iter().enumerate() {
        if value.get("cache_control").is_some() {
            paths.push(path(index));
        }
    }
}

fn remove_cache_directive(body: &mut serde_json::Value, path: CacheDirectivePath) {
    let target = match path {
        CacheDirectivePath::Tool(index) => body["tools"].get_mut(index),
        CacheDirectivePath::System(index) => body["system"].get_mut(index),
        CacheDirectivePath::MessageBlock(message, block) => body["messages"]
            .get_mut(message)
            .and_then(|value| value.get_mut("content"))
            .and_then(|value| value.get_mut(block)),
    };
    if let Some(object) = target.and_then(serde_json::Value::as_object_mut) {
        object.remove("cache_control");
    }
}

pub(crate) fn cached_tool_blocks(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut tools: Vec<serde_json::Value> = tools.to_vec();
    if let Some(last) = tools.last_mut()
        && let Some(obj) = last.as_object_mut()
        && !obj.contains_key("cache_control")
    {
        obj.insert(
            "cache_control".into(),
            serde_json::json!({ "type": "ephemeral" }),
        );
    }
    tools
}

pub(crate) fn extract_unknown_beta(body: &str) -> Option<String> {
    const MARKER: &str = "Unknown beta flag: ";
    let start = body.find(MARKER)? + MARKER.len();
    let rest = &body[start..];
    let end = rest.find('"').unwrap_or(rest.len());
    let name = rest[..end].trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

pub(crate) fn classify_error(
    status: u16,
    body: &str,
    retry_after_header: Option<&str>,
) -> ApiError {
    match status {
        401 => ApiError::AuthError(format!("authentication failed: {body}")),
        403 => ApiError::AuthError(format!(
            "authentication/identity rejected (403). If using spoof mode, check \
             identity.spoof_version matches the current Claude Code version, or \
             run /refresh-identity to rediscover beta headers. Body: {body}"
        )),
        429 => ApiError::RateLimited {
            retry_after_secs: retry_after_header
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| extract_retry_after(body)),
        },
        529 => ApiError::Overloaded,
        500 | 502 | 503 => ApiError::ServerError {
            status,
            message: body.to_string(),
        },
        _ => ApiError::HttpError(format!("HTTP {status}: {body}")),
    }
}

fn extract_retry_after(body: &str) -> u64 {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(secs) = v.get("retry_after").and_then(|v| v.as_u64())
    {
        return secs;
    }
    30
}
