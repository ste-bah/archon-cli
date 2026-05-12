use crate::provider::{LlmError, LlmProvider};

pub const FALLBACK_CONTEXT_WINDOW: u64 = 200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextWindowSource {
    ConfigOverride,
    ProviderMetadata,
    KnownModel,
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextWindowResolution {
    pub model: String,
    pub context_window: u64,
    pub source: ContextWindowSource,
}

pub fn classify_context_window_error(
    status: Option<u16>,
    error_type: Option<&str>,
    code: Option<&str>,
    message: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> Option<LlmError> {
    let mut haystack = String::new();
    if let Some(error_type) = error_type {
        haystack.push_str(error_type);
        haystack.push(' ');
    }
    if let Some(code) = code {
        haystack.push_str(code);
        haystack.push(' ');
    }
    haystack.push_str(message);
    let lower = haystack.to_ascii_lowercase();

    let status_matches = status.is_none_or(|s| matches!(s, 400 | 413 | 422));
    let text_matches = [
        "context_length_exceeded",
        "context window",
        "context length",
        "maximum context",
        "max context",
        "prompt is too long",
        "prompt too long",
        "too many tokens",
        "token limit",
        "input is too long",
        "request too large",
    ]
    .iter()
    .any(|needle| lower.contains(needle));

    if status_matches && text_matches {
        Some(LlmError::ContextWindowExceeded {
            provider_message: message.to_string(),
            provider: provider.map(str::to_string),
            model: model.map(str::to_string),
        })
    } else {
        None
    }
}

pub fn classify_context_window_body(
    status: u16,
    body: &str,
    provider: Option<&str>,
    model: Option<&str>,
) -> Option<LlmError> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        let error = value.get("error").unwrap_or(&value);
        let error_type = error
            .get("type")
            .or_else(|| error.get("error_type"))
            .and_then(|v| v.as_str());
        let code = error.get("code").and_then(|v| v.as_str());
        let message = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or(body);
        return classify_context_window_error(
            Some(status),
            error_type,
            code,
            message,
            provider,
            model,
        );
    }

    classify_context_window_error(Some(status), None, None, body, provider, model)
}

pub fn resolve_context_window(
    active_model: &str,
    override_window: Option<u64>,
    provider: Option<&dyn LlmProvider>,
) -> ContextWindowResolution {
    if let Some(window) = override_window.filter(|w| *w > 0) {
        return ContextWindowResolution {
            model: active_model.to_string(),
            context_window: window,
            source: ContextWindowSource::ConfigOverride,
        };
    }

    let resolved_model = provider
        .and_then(|p| p.resolve_alias(active_model))
        .unwrap_or_else(|| active_model.to_string());

    if let Some(info) = provider
        .and_then(|p| p.models().into_iter().find(|m| m.id == resolved_model))
        .filter(|m| m.context_window > 0)
    {
        return ContextWindowResolution {
            model: info.id,
            context_window: info.context_window as u64,
            source: ContextWindowSource::ProviderMetadata,
        };
    }

    if let Some(window) = known_context_window(&resolved_model) {
        return ContextWindowResolution {
            model: resolved_model,
            context_window: window,
            source: ContextWindowSource::KnownModel,
        };
    }

    ContextWindowResolution {
        model: resolved_model,
        context_window: FALLBACK_CONTEXT_WINDOW,
        source: ContextWindowSource::Fallback,
    }
}

fn known_context_window(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") || m.contains("gpt-5") {
        Some(200_000)
    } else if m.contains("gpt-4o") || m.contains("gpt-4-turbo") {
        Some(128_000)
    } else if m.contains("llama") || m.contains("mistral") || m.contains("qwen") {
        Some(128_000)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_overflow_body_classifies() {
        let body = r#"{"error":{"code":"context_length_exceeded","message":"maximum context length exceeded"}}"#;
        assert!(classify_context_window_body(400, body, Some("openai"), Some("gpt")).is_some());
    }

    #[test]
    fn ordinary_bad_request_does_not_classify() {
        assert!(classify_context_window_body(400, "bad api key shape", None, None).is_none());
    }
}
