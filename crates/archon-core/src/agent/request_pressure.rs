#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestSizeBreakdown {
    pub total_body_bytes: usize,
    pub approx_tokens: u64,
    pub system_bytes: usize,
    pub messages_bytes: usize,
    pub tools_bytes: usize,
    pub extra_bytes: usize,
    pub message_count: usize,
    pub tool_count: usize,
}

pub(crate) fn request_body_bytes(request: &archon_llm::provider::LlmRequest) -> usize {
    let breakdown = request_size_breakdown(request);
    if request.request_origin.is_some() {
        tracing::info!(
            target: "archon::context",
            request_origin = request.request_origin.as_deref().unwrap_or("unknown"),
            request_model = %request.model,
            request_body_bytes = breakdown.total_body_bytes,
            request_approx_tokens = breakdown.approx_tokens,
            request_system_bytes = breakdown.system_bytes,
            request_messages_bytes = breakdown.messages_bytes,
            request_tools_bytes = breakdown.tools_bytes,
            request_extra_bytes = breakdown.extra_bytes,
            request_message_count = breakdown.message_count,
            request_tool_count = breakdown.tool_count,
            "llm request size preflight"
        );
        log_system_block_sizes(request);
    }
    breakdown.total_body_bytes
}

fn log_system_block_sizes(request: &archon_llm::provider::LlmRequest) {
    if request.system.is_empty() {
        return;
    }

    let mut blocks: Vec<(usize, usize, String)> = request
        .system
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let text = block
                .get("text")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let preview: String = text
                .chars()
                .take(80)
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .collect();
            (index, serialized_len(block), preview)
        })
        .collect();
    blocks.sort_by(|left, right| right.1.cmp(&left.1));

    for (rank, (index, bytes, preview)) in blocks.into_iter().take(8).enumerate() {
        tracing::info!(
            target: "archon::context",
            request_origin = request.request_origin.as_deref().unwrap_or("unknown"),
            request_model = %request.model,
            rank = rank + 1,
            system_block_index = index,
            system_block_bytes = bytes,
            system_block_preview = %preview,
            "llm request system block size"
        );
    }
}

pub(crate) fn request_size_breakdown(
    request: &archon_llm::provider::LlmRequest,
) -> RequestSizeBreakdown {
    let total_body_bytes = serialized_len(&serde_json::json!({
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
    }));

    RequestSizeBreakdown {
        total_body_bytes,
        approx_tokens: approx_tokens_from_bytes(total_body_bytes),
        system_bytes: serialized_len(&request.system),
        messages_bytes: serialized_len(&request.messages),
        tools_bytes: serialized_len(&request.tools),
        extra_bytes: serialized_len(&request.extra),
        message_count: request.messages.len(),
        tool_count: request.tools.len(),
    }
}

pub(crate) fn approx_tokens_from_bytes(bytes: usize) -> u64 {
    bytes.div_ceil(4) as u64
}

fn serialized_len(value: &impl serde::Serialize) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

pub(crate) fn large_request_retry_body_bytes(config: &crate::config::ContextConfig) -> usize {
    config
        .large_request_retry_body_bytes
        .unwrap_or(320_000)
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

    #[test]
    fn request_size_breakdown_reports_major_components() {
        let request = LlmRequest {
            system: vec![serde_json::json!({"type": "text", "text": "sys"})],
            messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
            tools: vec![serde_json::json!({"name": "Agent", "description": "spawn"})],
            extra: serde_json::json!({"runtime": "test"}),
            ..LlmRequest::default()
        };

        let breakdown = request_size_breakdown(&request);

        assert_eq!(breakdown.message_count, 1);
        assert_eq!(breakdown.tool_count, 1);
        assert!(breakdown.system_bytes > 0);
        assert!(breakdown.messages_bytes > 0);
        assert!(breakdown.tools_bytes > 0);
        assert!(breakdown.total_body_bytes >= breakdown.messages_bytes);
        assert_eq!(
            breakdown.approx_tokens,
            approx_tokens_from_bytes(breakdown.total_body_bytes)
        );
    }

    #[test]
    fn large_request_retry_default_is_below_observed_freeze_size() {
        let config = crate::config::ContextConfig::default();

        assert_eq!(large_request_retry_body_bytes(&config), 320_000);
        assert!(
            large_request_retry_body_bytes(&config) < 636_725,
            "issue #42 request should be treated as large"
        );
    }
}
