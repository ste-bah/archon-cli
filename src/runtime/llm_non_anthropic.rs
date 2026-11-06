//! Non-Anthropic provider construction without requiring Anthropic fallback auth.

use std::sync::Arc;

use anyhow::Result;
use archon_core::config::LlmConfig;
use archon_llm::provider::LlmProvider;
use archon_llm::providers::{BedrockProvider, LocalProvider, OpenAiProvider, VertexProvider};
use archon_llm::{ActiveProvider, LlmConfig as FlatLlmConfig};

pub(crate) fn build_llm_provider_without_anthropic_fallback(
    llm_cfg: &LlmConfig,
) -> Result<Arc<dyn LlmProvider>> {
    match llm_cfg.provider.as_str() {
        "anthropic" => Err(anyhow::anyhow!(
            "anthropic provider requires Anthropic auth construction"
        )),
        "openai" => build_openai(llm_cfg),
        "bedrock" => build_bedrock(llm_cfg),
        "vertex" => build_vertex(llm_cfg),
        "local" => Ok(build_local(llm_cfg)),
        other => build_openai_compatible(other, llm_cfg),
    }
}

fn build_openai(llm_cfg: &LlmConfig) -> Result<Arc<dyn LlmProvider>> {
    let resolved = openai_api_key(llm_cfg);
    if resolved.is_empty() {
        anyhow::bail!("OpenAI selected but no API key found");
    }
    Ok(Arc::new(OpenAiProvider::with_resolved_api_key(
        resolved,
        llm_cfg.openai.base_url.clone(),
        llm_cfg.openai.model.clone(),
    )))
}

fn build_bedrock(llm_cfg: &LlmConfig) -> Result<Arc<dyn LlmProvider>> {
    let region = llm_cfg.bedrock.region.clone();
    let model_id = llm_cfg.bedrock.model_id.clone();
    if region.is_empty() || model_id.is_empty() {
        anyhow::bail!("Bedrock selected but region/model_id missing");
    }
    Ok(Arc::new(BedrockProvider::new(region, model_id)))
}

fn build_vertex(llm_cfg: &LlmConfig) -> Result<Arc<dyn LlmProvider>> {
    let project_id = llm_cfg.vertex.project_id.as_deref().unwrap_or("");
    if project_id.is_empty() {
        anyhow::bail!("Vertex selected but project_id missing");
    }
    let publisher = if llm_cfg.vertex.model.contains("claude") {
        "anthropic"
    } else {
        "google"
    };
    Ok(Arc::new(VertexProvider::new(
        project_id.to_string(),
        llm_cfg.vertex.region.clone(),
        llm_cfg.vertex.model.clone(),
        publisher.to_string(),
        llm_cfg.vertex.credentials_file.clone(),
    )))
}

fn build_local(llm_cfg: &LlmConfig) -> Arc<dyn LlmProvider> {
    Arc::new(
        LocalProvider::new(
            llm_cfg.local.base_url.clone(),
            llm_cfg.local.model.clone(),
            llm_cfg.local.timeout_secs,
            llm_cfg.local.pull_if_missing,
        )
        .with_reasoning(llm_cfg.local.reasoning.clone()),
    )
}

fn build_openai_compatible(provider: &str, llm_cfg: &LlmConfig) -> Result<Arc<dyn LlmProvider>> {
    let binding = llm_cfg.provider_environment.get(provider);
    let flat = FlatLlmConfig {
        provider: provider.to_string(),
        model: None,
        base_url: binding
            .and_then(|b| b.base_url.as_ref())
            .map(|value| value.parse())
            .transpose()?,
        api_key_env: binding.and_then(|b| b.api_key_env.clone()),
        retry: None,
    };
    let http = Arc::new(archon_llm::transport_http::client_builder().build()?);
    ActiveProvider::new(&flat, http)
        .map(|active| Arc::new(active) as Arc<dyn LlmProvider>)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

/// Resolve the same registry binding used for credential capture; never consult a second key.
pub(crate) fn openai_api_key(config: &LlmConfig) -> String {
    resolve_openai_api_key(config, |name| std::env::var(name).ok())
}

fn resolve_openai_api_key(
    config: &LlmConfig,
    lookup: impl FnOnce(&str) -> Option<String>,
) -> String {
    let full_config = archon_core::config::ArchonConfig {
        llm: config.clone(),
        ..Default::default()
    };
    let binding = crate::command::workflow_provider_route::bindings(&full_config)
        .remove(&config.provider)
        .expect("selected provider has a registry binding");
    binding
        .api_key_env
        .as_deref()
        .and_then(lookup)
        .or_else(|| config.openai.api_key.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_key_override_uses_only_the_captured_binding() {
        let mut config = LlmConfig {
            provider: "openai".into(),
            ..Default::default()
        };
        config.provider_environment.insert(
            "openai".into(),
            archon_core::config::ProviderEnvironmentConfig {
                api_key_env: Some("CUSTOM_KEY".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_openai_api_key(&config, |name| {
                assert_eq!(name, "CUSTOM_KEY");
                Some("override-key".into())
            }),
            "override-key"
        );
        config.provider_environment.clear();
        assert_eq!(
            resolve_openai_api_key(&config, |name| {
                assert_eq!(name, "OPENAI_API_KEY");
                Some("standard-key".into())
            }),
            "standard-key"
        );
    }
}
