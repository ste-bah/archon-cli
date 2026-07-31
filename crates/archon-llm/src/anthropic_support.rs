use std::collections::HashSet;
use std::sync::{Mutex as StdMutex, OnceLock};

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

pub(crate) fn effective_speed(request: &MessageRequest) -> Option<&str> {
    let value = request.speed.as_deref()?;
    if supports_speed(&request.model) {
        return Some(value);
    }
    warn_dropped_knob(&request.model, "speed", value);
    None
}

pub(crate) fn effective_effort(request: &MessageRequest) -> Option<&str> {
    let value = request.effort.as_deref()?;
    if supports_output_effort(&request.model) {
        return Some(value);
    }
    warn_dropped_knob(&request.model, "output_config.effort", value);
    None
}

fn supports_speed(_model: &str) -> bool {
    false
}

fn supports_output_effort(model: &str) -> bool {
    // Current-generation Claude models accept `output_config.effort`. Previously a
    // blanket `false` stub, which silently dropped the knob for every model; enabled
    // here for the models that support it (needed by the FCDP drafting pipeline, which
    // relies on effort:medium to cap runaway thinking). No other caller sets
    // `request.effort` today, so the blast radius is limited to opt-in effort requests.
    model.contains("opus-4") || model.contains("sonnet-5") || model.contains("fable-5")
}

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
