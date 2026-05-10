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
        other => build_openai_compatible(other),
    }
}

fn build_openai(llm_cfg: &LlmConfig) -> Result<Arc<dyn LlmProvider>> {
    let inline_key = llm_cfg.openai.api_key.clone().unwrap_or_default();
    let resolved = OpenAiProvider::resolve_api_key(&inline_key);
    if resolved.is_empty() {
        anyhow::bail!("OpenAI selected but no API key found");
    }
    Ok(Arc::new(OpenAiProvider::new(
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
    Arc::new(LocalProvider::new(
        llm_cfg.local.base_url.clone(),
        llm_cfg.local.model.clone(),
        llm_cfg.local.timeout_secs,
        llm_cfg.local.pull_if_missing,
    ))
}

fn build_openai_compatible(provider: &str) -> Result<Arc<dyn LlmProvider>> {
    let flat = FlatLlmConfig {
        provider: provider.to_string(),
        model: None,
        base_url: None,
        api_key_env: None,
        retry: None,
    };
    let http = Arc::new(reqwest::Client::new());
    ActiveProvider::new(&flat, http)
        .map(|active| Arc::new(active) as Arc<dyn LlmProvider>)
        .map_err(|error| anyhow::anyhow!("{error}"))
}
