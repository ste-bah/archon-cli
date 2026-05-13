use std::path::Path;

use crate::context_catalog::ContextCatalog;
use crate::identity::IdentityMode;
use crate::provider::{LlmError, LlmProvider, LlmRequest};

pub const FALLBACK_CONTEXT_WINDOW: u64 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextWindowSource {
    ConfigOverride,
    UserCatalog,
    BundledCatalog,
    ProviderMetadata,
    Fallback,
}

impl ContextWindowSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ConfigOverride => "config-override",
            Self::UserCatalog => "user-catalog",
            Self::BundledCatalog => "bundled-catalog",
            Self::ProviderMetadata => "provider",
            Self::Fallback => "fallback",
        }
    }
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
        "too many input tokens",
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
    let cwd = std::env::current_dir().ok();
    resolve_context_window_for_work_dir(active_model, override_window, provider, cwd.as_deref())
}

pub fn resolve_context_window_for_work_dir(
    active_model: &str,
    override_window: Option<u64>,
    provider: Option<&dyn LlmProvider>,
    work_dir: Option<&Path>,
) -> ContextWindowResolution {
    if let Some(window) = override_window.filter(|w| *w > 0) {
        return ContextWindowResolution {
            model: active_model.to_string(),
            context_window: window,
            source: ContextWindowSource::ConfigOverride,
        };
    }

    let resolved_model = provider
        .map(|p| {
            let mut request = LlmRequest {
                model: active_model.to_string(),
                ..LlmRequest::default()
            };
            p.resolve_request_model(&mut request);
            request.model
        })
        .unwrap_or_else(|| active_model.to_string());

    let active_betas = active_provider_betas(provider);
    let active_identity = active_provider_identity(provider);
    let user_catalog = ContextCatalog::user_overrides(work_dir);
    if let Some(entry) = lookup_catalog(
        &user_catalog,
        provider,
        &resolved_model,
        &active_betas,
        &active_identity,
    ) {
        return ContextWindowResolution {
            model: resolved_model,
            context_window: entry.context_window,
            source: ContextWindowSource::UserCatalog,
        };
    }

    let bundled_catalog = ContextCatalog::bundled();
    if let Some(entry) = lookup_catalog(
        &bundled_catalog,
        provider,
        &resolved_model,
        &active_betas,
        &active_identity,
    ) {
        return ContextWindowResolution {
            model: resolved_model,
            context_window: entry.context_window,
            source: ContextWindowSource::BundledCatalog,
        };
    }

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

    ContextWindowResolution {
        model: resolved_model,
        context_window: FALLBACK_CONTEXT_WINDOW,
        source: ContextWindowSource::Fallback,
    }
}

fn lookup_catalog(
    catalog: &ContextCatalog,
    provider: Option<&dyn LlmProvider>,
    model: &str,
    active_betas: &[String],
    active_identity: &Option<String>,
) -> Option<crate::context_catalog::ContextWindowEntry> {
    provider
        .and_then(|p| catalog.lookup(p.name(), model, active_betas, active_identity.as_deref()))
        .or_else(|| catalog.lookup_any(model, active_betas, active_identity.as_deref()))
}

fn active_provider_betas(provider: Option<&dyn LlmProvider>) -> Vec<String> {
    provider
        .and_then(|p| p.as_anthropic())
        .and_then(|client| {
            client
                .identity()
                .request_headers("context-window-resolution")
                .get("anthropic-beta")
                .cloned()
        })
        .map(|header| {
            header
                .split(',')
                .map(str::trim)
                .filter(|beta| !beta.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn active_provider_identity(provider: Option<&dyn LlmProvider>) -> Option<String> {
    provider.and_then(|p| {
        p.as_anthropic()
            .map(|client| match &client.identity().mode {
                IdentityMode::Spoof { .. } => "spoof".to_string(),
                IdentityMode::Clean => "clean".to_string(),
                IdentityMode::Custom { .. } => "custom".to_string(),
            })
    })
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

    #[test]
    fn config_override_source_has_stable_label() {
        let resolved = resolve_context_window("anything", Some(123), None);
        assert_eq!(resolved.context_window, 123);
        assert_eq!(resolved.source, ContextWindowSource::ConfigOverride);
        assert_eq!(resolved.source.label(), "config-override");
    }

    #[test]
    fn unknown_model_uses_fallback_source() {
        let resolved = resolve_context_window("not-in-catalog", None, None);
        assert_eq!(resolved.context_window, FALLBACK_CONTEXT_WINDOW);
        assert_eq!(resolved.source, ContextWindowSource::Fallback);
        assert_eq!(resolved.source.label(), "fallback");
    }
}
