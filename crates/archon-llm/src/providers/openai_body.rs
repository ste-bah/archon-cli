use super::*;

/// As [`build_openai_request_body`], with an optional prompt-cache placement.
///
/// `prompt_cache_options` is sent only for `explicit` mode, because it turns
/// OpenAI's own implicit breakpoints **off**. In `hybrid` the breakpoint is
/// added alongside them, so a misjudged placement costs nothing rather than
/// costing the caching that would otherwise have happened by itself.
pub fn build_openai_request_body_cached(
    model: &str,
    max_tokens: u32,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    stream: bool,
    cache: Option<&crate::cache_wire::OpenAiCachePlacement>,
) -> serde_json::Value {
    let openai_messages = OpenAiProvider::build_openai_messages_cached(system, messages, cache);
    let openai_tools = OpenAiProvider::map_tools_to_openai(tools);

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": openai_messages,
        "stream": stream
    });

    if !openai_tools.is_empty() {
        body["tools"] = serde_json::Value::Array(openai_tools);
    }

    if let Some(cache) = cache {
        body["prompt_cache_key"] = serde_json::json!(cache.cache_key);
        if cache.explicit_only {
            body["prompt_cache_options"] = serde_json::json!({ "mode": "explicit" });
        }
    }

    body
}

pub fn build_openai_stream_request_body(
    model: &str,
    max_tokens: u32,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
) -> serde_json::Value {
    build_openai_stream_request_body_cached(model, max_tokens, system, messages, tools, None)
}

pub fn build_openai_stream_request_body_cached(
    model: &str,
    max_tokens: u32,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    cache: Option<&crate::cache_wire::OpenAiCachePlacement>,
) -> serde_json::Value {
    let mut body =
        build_openai_request_body_cached(model, max_tokens, system, messages, tools, true, cache);
    body["stream_options"] = serde_json::json!({"include_usage": true});
    body
}

// SSE parsing lives in `openai_stream`; re-exported here so existing
// `providers::openai::parse_openai_sse_chunk` call sites keep working.

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------
