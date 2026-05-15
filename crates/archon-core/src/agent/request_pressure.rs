pub(crate) fn request_body_bytes(request: &archon_llm::provider::LlmRequest) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "model": &request.model,
        "max_tokens": request.max_tokens,
        "system": &request.system,
        "messages": &request.messages,
        "tools": &request.tools,
        "thinking": &request.thinking,
        "speed": &request.speed,
        "effort": &request.effort,
        "extra": &request.extra,
        "request_origin": &request.request_origin,
        "reasoning_encrypted": &request.reasoning_encrypted,
    }))
    .map(|bytes| bytes.len())
    .unwrap_or(usize::MAX)
}

pub(crate) fn large_request_retry_body_bytes(config: &crate::config::ContextConfig) -> usize {
    config
        .large_request_retry_body_bytes
        .unwrap_or(1_000_000)
        .try_into()
        .unwrap_or(usize::MAX)
}

pub(crate) fn is_rate_limited_error(error: &archon_llm::provider::LlmError) -> bool {
    match error {
        archon_llm::provider::LlmError::RateLimited { .. } => true,
        archon_llm::provider::LlmError::Http(message)
        | archon_llm::provider::LlmError::Server { message, .. } => {
            let lower = message.to_ascii_lowercase();
            lower.contains("429") || lower.contains("rate limit") || lower.contains("rate_limited")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use archon_llm::provider::{LlmError, LlmRequest};

    use super::*;

    #[test]
    fn rate_limit_classifier_covers_typed_and_http_errors() {
        assert!(is_rate_limited_error(&LlmError::RateLimited {
            retry_after_secs: 30
        }));
        assert!(is_rate_limited_error(&LlmError::Http(
            "HTTP 429: rate_limited".into()
        )));
        assert!(!is_rate_limited_error(&LlmError::Http(
            "ordinary bad request".into()
        )));
    }

    #[test]
    fn request_body_bytes_includes_messages_and_tools() {
        let request = LlmRequest {
            messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
            tools: vec![serde_json::json!({"name": "Agent", "description": "spawn"})],
            ..LlmRequest::default()
        };

        assert!(request_body_bytes(&request) > 100);
    }
}
