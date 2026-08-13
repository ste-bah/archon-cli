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
    pub runtime_context_budget: Option<u64>,
    pub source: ContextWindowSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestPressureKind {
    ToolResultField,
    AggregateContext,
    OpeningPrompt,
}

pub fn classify_request_pressure_error(
    status: Option<u16>,
    error_type: Option<&str>,
    code: Option<&str>,
    message: &str,
) -> Option<RequestPressureKind> {
    let status_matches = status.is_none_or(|value| matches!(value, 400 | 413 | 422));
    if !status_matches {
        return None;
    }
    let mut text = String::new();
    if let Some(error_type) = error_type {
        text.push_str(error_type);
        text.push(' ');
    }
    if let Some(code) = code {
        text.push_str(code);
        text.push(' ');
    }
    text.push_str(message);
    let lower = text.to_ascii_lowercase();
    let field_limit = [
        "string should have at most",
        "string too long",
        "maximum length",
        "max string length",
        "field too long",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if field_limit
        && ["tool_result", "tool result", ".output", ".content"]
            .iter()
            .any(|needle| lower.contains(needle))
    {
        return Some(RequestPressureKind::ToolResultField);
    }
    classify_context_window_error(status, error_type, code, message, None, None)
        .map(|_| RequestPressureKind::AggregateContext)
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
            runtime_context_budget: None,
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
            runtime_context_budget: entry.runtime_context_budget,
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
            runtime_context_budget: entry.runtime_context_budget,
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
            runtime_context_budget: None,
            source: ContextWindowSource::ProviderMetadata,
        };
    }

    ContextWindowResolution {
        model: resolved_model,
        context_window: FALLBACK_CONTEXT_WINDOW,
        runtime_context_budget: None,
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
        .or_else(|| {
            // Bedrock and Vertex address the same Claude models under decorated
            // ids — `eu.anthropic.claude-sonnet-4-6`,
            // `us.anthropic.claude-sonnet-4-5-20250929-v1:0`, or a full
            // inference-profile ARN. The catalog is keyed on the bare
            // `claude-sonnet-4-6`, and there is no `[providers.bedrock]` section
            // at all, so both lookups above miss and the window resolves to
            // unknown.
            //
            // That is not cosmetic. The status bar showing `ctx 32k/?` is the
            // visible half; the costly half is that the context window is what
            // sizes auto-compaction, so an unknown window means compaction
            // cannot be triggered against a real budget.
            let bare = bare_model_id(model);
            if bare == model {
                return None;
            }
            catalog
                .lookup_any(&bare, active_betas, active_identity.as_deref())
                .or_else(|| {
                    // Some catalogue keys carry the release date
                    // (`claude-haiku-4-5-20251001`) and some do not, so try
                    // without a trailing date too.
                    let undated = strip_release_date(&bare);
                    (undated != bare)
                        .then(|| {
                            catalog.lookup_any(&undated, active_betas, active_identity.as_deref())
                        })
                        .flatten()
                })
        })
}

/// Reduce a decorated provider model id to the bare model name.
///
/// `arn:aws:bedrock:eu-west-2:123456789012:inference-profile/eu.anthropic.claude-sonnet-4-6-v1:0`
/// and `eu.anthropic.claude-sonnet-4-6` both become `claude-sonnet-4-6`.
fn bare_model_id(model: &str) -> String {
    // An ARN puts the profile id after the last '/'.
    let id = model.rsplit_once('/').map_or(model, |(_, tail)| tail);
    // Cross-region inference profiles carry a geo prefix.
    let id = ["us-gov.", "us.", "eu.", "apac."]
        .iter()
        .find_map(|geo| id.strip_prefix(geo))
        .unwrap_or(id);
    // Then the vendor prefix.
    let id = ["anthropic.", "openai.", "amazon.", "meta."]
        .iter()
        .find_map(|vendor| id.strip_prefix(vendor))
        .unwrap_or(id);
    // Finally the Bedrock version suffix, `-v1:0`.
    match id.rfind("-v") {
        Some(pos)
            if id[pos + 2..]
                .chars()
                .all(|c| c.is_ascii_digit() || c == ':') =>
        {
            id[..pos].to_string()
        }
        _ => id.to_string(),
    }
}

/// Drop a trailing `-YYYYMMDD` release date, if present.
fn strip_release_date(model: &str) -> String {
    match model.rfind('-') {
        Some(pos)
            if model.len() - pos == 9 && model[pos + 1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            model[..pos].to_string()
        }
        _ => model.to_string(),
    }
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
    fn provider_field_length_rejection_classifies_separately_from_aggregate_overflow() {
        let field = classify_request_pressure_error(
            Some(400),
            Some("invalid_request_error"),
            None,
            "messages.3.content.0.tool_result.content: String should have at most 10485760 characters",
        );
        assert_eq!(field, Some(RequestPressureKind::ToolResultField));

        let aggregate = classify_request_pressure_error(
            Some(400),
            Some("invalid_request_error"),
            Some("context_length_exceeded"),
            "maximum context length exceeded",
        );
        assert_eq!(aggregate, Some(RequestPressureKind::AggregateContext));
    }

    #[test]
    fn ordinary_bad_request_is_not_request_pressure() {
        assert_eq!(
            classify_request_pressure_error(
                Some(400),
                Some("invalid_request_error"),
                None,
                "invalid bearer token shape",
            ),
            None
        );
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
