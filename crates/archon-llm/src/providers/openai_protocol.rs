use crate::provider::LlmError;
use crate::streaming::StreamEvent;
use crate::types::Usage;

pub(super) fn usage_event(usage: Usage) -> Vec<StreamEvent> {
    vec![StreamEvent::MessageDelta {
        stop_reason: None,
        usage: Some(usage),
    }]
}

pub(super) fn usage_from_openai_chunk(value: &serde_json::Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    Some(Usage {
        input_tokens: usage
            .get("prompt_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("completion_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        input_tokens_available: usage
            .get("prompt_tokens")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        output_tokens_available: usage
            .get("completion_tokens")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        cache_creation_input_tokens_available: false,
        cache_read_input_tokens_available: false,
    })
}

pub(super) fn map_http_error(status: u16, body: String) -> LlmError {
    if let Some(err) =
        crate::context_window::classify_context_window_body(status, &body, Some("openai"), None)
    {
        return err;
    }
    match status {
        401 => LlmError::Auth(body),
        429 => LlmError::RateLimited {
            retry_after_secs: 60,
        },
        500 | 503 => LlmError::Overloaded,
        _ => LlmError::Server {
            status,
            message: body,
        },
    }
}
