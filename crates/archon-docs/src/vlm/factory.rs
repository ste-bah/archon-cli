use serde::Deserialize;

use crate::errors::DocsError;
use crate::vlm::anthropic::{AnthropicVlmProvider, DEFAULT_ANTHROPIC_VLM_MODEL};
use crate::vlm::gemini::{DEFAULT_GEMINI_MODEL, GeminiVlmProvider};
use crate::vlm::ollama::{DEFAULT_OLLAMA_MODEL, OllamaVlmProvider};
use crate::vlm::openai_compat::{DEFAULT_OPENAI_COMPAT_MODEL, OpenAiCompatVlmProvider};
use crate::vlm::{
    RegisteredVlmProvider, VlmProviderMetadata, clear_provider, set_registered_provider,
};

#[derive(Clone, Debug, PartialEq)]
pub enum VlmProviderInitStatus {
    Registered,
    Disabled,
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VlmProviderInitReport {
    pub status: VlmProviderInitStatus,
    pub provider: String,
    pub model: String,
    pub message: String,
}

impl VlmProviderInitReport {
    fn registered(provider: impl Into<String>, model: impl Into<String>) -> Self {
        let provider = provider.into();
        let model = model.into();
        Self {
            status: VlmProviderInitStatus::Registered,
            message: format!("VLM provider registered: {provider}/{model}"),
            provider,
            model,
        }
    }

    fn disabled(message: impl Into<String>) -> Self {
        Self {
            status: VlmProviderInitStatus::Disabled,
            provider: "disabled".into(),
            model: String::new(),
            message: message.into(),
        }
    }

    fn skipped(
        provider: impl Into<String>,
        model: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: VlmProviderInitStatus::Skipped,
            provider: provider.into(),
            model: model.into(),
            message: message.into(),
        }
    }
}

pub fn configure_registered_provider(
    policy: &archon_policy::EffectivePolicy,
) -> VlmProviderInitReport {
    clear_provider();

    let decision = policy.docs_vlm_decision();
    if !decision.allowed {
        return VlmProviderInitReport::disabled(decision.reason);
    }

    match policy.docs.vlm.provider.as_str() {
        "ollama" => configure_ollama(policy),
        "gemini" => configure_gemini(policy),
        "anthropic" => configure_anthropic(policy),
        "openai-compat" => configure_openai_compat(policy),
        "disabled" | "" => VlmProviderInitReport::disabled("VLM provider disabled by policy"),
        other => {
            VlmProviderInitReport::skipped(other, "", format!("unsupported VLM provider '{other}'"))
        }
    }
}

pub async fn configure_registered_provider_blocking_safe(
    policy: &archon_policy::EffectivePolicy,
) -> VlmProviderInitReport {
    if tokio::runtime::Handle::try_current().is_ok() {
        let policy = policy.clone();
        let fallback_provider = policy.docs.vlm.provider.clone();
        let fallback_model = configured_model_label(&policy);
        match tokio::task::spawn_blocking(move || configure_registered_provider(&policy)).await {
            Ok(report) => report,
            Err(e) => VlmProviderInitReport::skipped(
                fallback_provider,
                fallback_model,
                format!("VLM provider initialization task failed: {e}"),
            ),
        }
    } else {
        configure_registered_provider(policy)
    }
}

pub fn configure_registered_provider_thread_safe(
    policy: &archon_policy::EffectivePolicy,
) -> VlmProviderInitReport {
    if tokio::runtime::Handle::try_current().is_err() {
        return configure_registered_provider(policy);
    }

    let policy = policy.clone();
    let fallback_provider = policy.docs.vlm.provider.clone();
    let fallback_model = configured_model_label(&policy);
    match std::thread::Builder::new()
        .name("archon-vlm-init".into())
        .spawn(move || configure_registered_provider(&policy))
    {
        Ok(handle) => match handle.join() {
            Ok(report) => report,
            Err(_) => VlmProviderInitReport::skipped(
                fallback_provider,
                fallback_model,
                "VLM provider initialization thread panicked",
            ),
        },
        Err(e) => VlmProviderInitReport::skipped(
            fallback_provider,
            fallback_model,
            format!("VLM provider initialization thread failed: {e}"),
        ),
    }
}

fn configured_model_label(policy: &archon_policy::EffectivePolicy) -> String {
    match policy.docs.vlm.provider.as_str() {
        "ollama" => policy.docs.vlm.ollama.model.clone(),
        "gemini" => policy.docs.vlm.gemini.model.clone(),
        "anthropic" => policy.docs.vlm.anthropic.model.clone(),
        "openai-compat" => policy.docs.vlm.openai_compat.model.clone(),
        _ => String::new(),
    }
}

pub fn diagnostic_report(policy: &archon_policy::EffectivePolicy) -> VlmProviderInitReport {
    let decision = policy.docs_vlm_decision();
    if !decision.allowed {
        return VlmProviderInitReport::disabled(decision.reason);
    }
    match policy.docs.vlm.provider.as_str() {
        "ollama" => {
            let provider = match OllamaVlmProvider::from_policy(&policy.docs.vlm.ollama) {
                Ok(provider) => provider,
                Err(e) => {
                    return VlmProviderInitReport::skipped(
                        "ollama",
                        policy.docs.vlm.ollama.model.clone(),
                        e.to_string(),
                    );
                }
            };
            match provider.health_check() {
                Ok(()) => VlmProviderInitReport::registered("ollama", provider.model()),
                Err(e) => VlmProviderInitReport::skipped("ollama", provider.model(), e.to_string()),
            }
        }
        "gemini" => {
            let Some(api_key) = resolve_google_api_key(&policy.docs.vlm.gemini.api_key_env) else {
                return VlmProviderInitReport::skipped(
                    "gemini",
                    policy.docs.vlm.gemini.model.clone(),
                    google_api_key_missing_message(&policy.docs.vlm.gemini.api_key_env),
                );
            };
            let provider = match GeminiVlmProvider::from_policy(&policy.docs.vlm.gemini, api_key) {
                Ok(provider) => provider,
                Err(e) => {
                    return VlmProviderInitReport::skipped(
                        "gemini",
                        policy.docs.vlm.gemini.model.clone(),
                        e.to_string(),
                    );
                }
            };
            match provider.health_check() {
                Ok(()) => VlmProviderInitReport::registered("gemini", provider.model()),
                Err(e) => VlmProviderInitReport::skipped("gemini", provider.model(), e.to_string()),
            }
        }
        "anthropic" => match build_anthropic_provider(policy) {
            Ok(provider) => VlmProviderInitReport::registered("anthropic", provider.model()),
            Err(e) => VlmProviderInitReport::skipped(
                "anthropic",
                policy.docs.vlm.anthropic.model.clone(),
                e.to_string(),
            ),
        },
        "openai-compat" => {
            let provider = match build_openai_compat_provider(policy) {
                Ok(provider) => provider,
                Err(e) => {
                    return VlmProviderInitReport::skipped(
                        "openai-compat",
                        policy.docs.vlm.openai_compat.model.clone(),
                        e.to_string(),
                    );
                }
            };
            match provider.health_check() {
                Ok(()) => VlmProviderInitReport::registered("openai-compat", provider.model()),
                Err(e) => {
                    VlmProviderInitReport::skipped("openai-compat", provider.model(), e.to_string())
                }
            }
        }
        "disabled" | "" => VlmProviderInitReport::disabled("VLM provider disabled by policy"),
        other => {
            VlmProviderInitReport::skipped(other, "", format!("unsupported VLM provider '{other}'"))
        }
    }
}

fn configure_ollama(policy: &archon_policy::EffectivePolicy) -> VlmProviderInitReport {
    let provider = match OllamaVlmProvider::from_policy(&policy.docs.vlm.ollama) {
        Ok(provider) => provider,
        Err(e) => {
            return VlmProviderInitReport::skipped(
                "ollama",
                policy.docs.vlm.ollama.model.clone(),
                e.to_string(),
            );
        }
    };
    let provider_id = provider.provider_id();
    let model = provider.model().to_string();
    if let Err(e) = provider.health_check() {
        tracing::warn!(
            provider = provider_id,
            model = %model,
            error = %e,
            "vlm provider health check failed; image descriptions will be skipped"
        );
        return VlmProviderInitReport::skipped(provider_id, model, e.to_string());
    }
    set_registered_provider(RegisteredVlmProvider::new(
        VlmProviderMetadata::new(provider_id, model.clone(), 0.0),
        Box::new(provider),
    ));
    VlmProviderInitReport::registered(provider_id, model)
}

fn configure_gemini(policy: &archon_policy::EffectivePolicy) -> VlmProviderInitReport {
    let Some(api_key) = resolve_google_api_key(&policy.docs.vlm.gemini.api_key_env) else {
        return VlmProviderInitReport::skipped(
            "gemini",
            policy.docs.vlm.gemini.model.clone(),
            google_api_key_missing_message(&policy.docs.vlm.gemini.api_key_env),
        );
    };
    let provider = match GeminiVlmProvider::from_policy(&policy.docs.vlm.gemini, api_key) {
        Ok(provider) => provider,
        Err(e) => {
            return VlmProviderInitReport::skipped(
                "gemini",
                policy.docs.vlm.gemini.model.clone(),
                e.to_string(),
            );
        }
    };
    let provider_id = provider.provider_id();
    let model = provider.model().to_string();
    if let Err(e) = provider.health_check() {
        tracing::warn!(
            provider = provider_id,
            model = %model,
            error = %e,
            "vlm provider health check failed; image descriptions will be skipped"
        );
        return VlmProviderInitReport::skipped(provider_id, model, e.to_string());
    }
    set_registered_provider(RegisteredVlmProvider::new(
        VlmProviderMetadata::new(provider_id, model.clone(), 0.0),
        Box::new(provider),
    ));
    VlmProviderInitReport::registered(provider_id, model)
}

fn configure_anthropic(policy: &archon_policy::EffectivePolicy) -> VlmProviderInitReport {
    match build_anthropic_provider(policy) {
        Ok(provider) => {
            let provider_id = provider.provider_id();
            let model = provider.model().to_string();
            set_registered_provider(RegisteredVlmProvider::new(
                VlmProviderMetadata::new(provider_id, model.clone(), 0.0),
                Box::new(provider),
            ));
            VlmProviderInitReport::registered(provider_id, model)
        }
        Err(e) => VlmProviderInitReport::skipped(
            "anthropic",
            policy.docs.vlm.anthropic.model.clone(),
            e.to_string(),
        ),
    }
}

fn configure_openai_compat(policy: &archon_policy::EffectivePolicy) -> VlmProviderInitReport {
    let provider = match build_openai_compat_provider(policy) {
        Ok(provider) => provider,
        Err(e) => {
            return VlmProviderInitReport::skipped(
                "openai-compat",
                policy.docs.vlm.openai_compat.model.clone(),
                e.to_string(),
            );
        }
    };
    let provider_id = provider.provider_id();
    let model = provider.model().to_string();
    if let Err(e) = provider.health_check() {
        tracing::warn!(
            provider = provider_id,
            model = %model,
            endpoint = %provider.endpoint(),
            error = %e,
            "vlm provider health check failed; image descriptions will be skipped"
        );
        return VlmProviderInitReport::skipped(provider_id, model, e.to_string());
    }
    set_registered_provider(RegisteredVlmProvider::new(
        VlmProviderMetadata::new(provider_id, model.clone(), 0.0),
        Box::new(provider),
    ));
    VlmProviderInitReport::registered(provider_id, model)
}

fn build_anthropic_provider(
    policy: &archon_policy::EffectivePolicy,
) -> Result<AnthropicVlmProvider, DocsError> {
    let auth =
        archon_llm::auth::resolve_auth_from_env().map_err(|e| DocsError::VlmAuthentication {
            provider: "anthropic".into(),
            message: e.to_string(),
        })?;
    let identity_mode = archon_llm::identity::resolve_identity_mode(
        &auth,
        false,
        &archon_llm::identity::IdentityConfigView::default(),
    );
    let identity = archon_llm::identity::IdentityProvider::new(
        identity_mode,
        format!("vlm-{}", uuid::Uuid::new_v4()),
        archon_llm::identity::get_or_create_device_id(),
        String::new(),
    );
    AnthropicVlmProvider::new(
        auth,
        identity,
        configured_anthropic_model(policy),
        std::env::var("ANTHROPIC_BASE_URL").ok(),
    )
}

fn build_openai_compat_provider(
    policy: &archon_policy::EffectivePolicy,
) -> Result<OpenAiCompatVlmProvider, DocsError> {
    OpenAiCompatVlmProvider::from_policy(
        &policy.docs.vlm.openai_compat,
        resolve_optional_env_key(&policy.docs.vlm.openai_compat.api_key_env),
    )
}

fn configured_anthropic_model(policy: &archon_policy::EffectivePolicy) -> String {
    let model = policy.docs.vlm.anthropic.model.trim();
    if model.is_empty() {
        DEFAULT_ANTHROPIC_VLM_MODEL.into()
    } else {
        model.into()
    }
}

fn resolve_optional_env_key(api_key_env: &str) -> Option<String> {
    if api_key_env.trim().is_empty() {
        return None;
    }
    std::env::var(api_key_env)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub fn resolve_google_api_key(api_key_env: &str) -> Option<String> {
    if let Ok(value) = std::env::var(api_key_env)
        && !value.trim().is_empty()
    {
        return Some(value);
    }
    let path = archon_llm::tokens::credentials_path();
    let json = std::fs::read_to_string(path).ok()?;
    google_api_key_from_json(&json)
}

fn google_api_key_missing_message(api_key_env: &str) -> String {
    format!(
        "Google API key missing; set {api_key_env} or run `archon auth login --provider google`"
    )
}

pub fn google_api_key_from_json(json: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct GoogleCredentialFile {
        #[serde(rename = "googleApiKey")]
        google_api_key: Option<String>,
    }

    serde_json::from_str::<GoogleCredentialFile>(json)
        .ok()
        .and_then(|file| file.google_api_key)
        .filter(|value| !value.trim().is_empty())
}

pub fn default_provider_summary(policy: &archon_policy::EffectivePolicy) -> (String, String) {
    match policy.docs.vlm.provider.as_str() {
        "ollama" => (
            "ollama".into(),
            non_empty_or_default(&policy.docs.vlm.ollama.model, DEFAULT_OLLAMA_MODEL),
        ),
        "gemini" => (
            "gemini".into(),
            non_empty_or_default(&policy.docs.vlm.gemini.model, DEFAULT_GEMINI_MODEL),
        ),
        "anthropic" => (
            "anthropic".into(),
            non_empty_or_default(
                &policy.docs.vlm.anthropic.model,
                DEFAULT_ANTHROPIC_VLM_MODEL,
            ),
        ),
        "openai-compat" => (
            "openai-compat".into(),
            non_empty_or_default(
                &policy.docs.vlm.openai_compat.model,
                DEFAULT_OPENAI_COMPAT_MODEL,
            ),
        ),
        other => (other.into(), String::new()),
    }
}

fn non_empty_or_default(value: &str, default: &str) -> String {
    if value.trim().is_empty() {
        default.into()
    } else {
        value.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_api_key_from_json_preserves_other_credentials() {
        let json = r#"{
            "claudeAiOauth": {"accessToken": "a"},
            "openaiCodexOauth": {"accessToken": "b"},
            "googleApiKey": "AIza-test"
        }"#;

        assert_eq!(google_api_key_from_json(json), Some("AIza-test".into()));
    }

    #[test]
    fn default_provider_summary_reads_nested_provider_model() {
        let mut policy = archon_policy::EffectivePolicy::default();
        policy.docs.vlm.provider = "ollama".into();
        policy.docs.vlm.ollama.model = "llava:13b".into();

        assert_eq!(
            default_provider_summary(&policy),
            ("ollama".into(), "llava:13b".into())
        );
    }

    #[test]
    fn default_provider_summary_reads_openai_compat_model() {
        let mut policy = archon_policy::EffectivePolicy::default();
        policy.docs.vlm.provider = "openai-compat".into();
        policy.docs.vlm.openai_compat.model = "vision-model".into();

        assert_eq!(
            default_provider_summary(&policy),
            ("openai-compat".into(), "vision-model".into())
        );
    }
}
