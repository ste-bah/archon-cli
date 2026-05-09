//! Codex provider construction behind the runtime strategy gate.

use std::sync::Arc;

use anyhow::{Context, Result};
use archon_core::config::ArchonConfig;
use archon_llm::provider::LlmProvider;

pub(crate) async fn build_codex_provider(
    config: &ArchonConfig,
    surface: &str,
) -> Result<(Arc<dyn LlmProvider>, &'static str)> {
    let decision = crate::runtime::codex_strategy::resolve_codex_runtime_strategy(
        &config.providers.openai_codex,
        surface,
    )?;
    let provider = build_direct_codex_provider(config, surface).await?;
    Ok((provider, decision.selected_runtime_mode))
}

async fn build_direct_codex_provider(
    config: &ArchonConfig,
    surface: &str,
) -> Result<Arc<dyn LlmProvider>> {
    let codex_cfg = crate::command::auth::codex_config_from_core(&config.providers.openai_codex);
    let http = reqwest::Client::new();
    let resolution = archon_llm::providers::codex::spoof::resolve(&codex_cfg, &http)
        .await
        .with_context(|| format!("failed to resolve Codex spoof identity for {surface}"))?;
    let provider = match std::env::var("ARCHON_CODEX_BASE_URL").ok() {
        Some(base_url) if !base_url.trim().is_empty() => {
            archon_llm::providers::codex::client::CodexProvider::new_with_base_url(
                archon_llm::tokens::credentials_path(),
                resolution.config,
                http,
                base_url,
            )
        }
        _ => archon_llm::providers::codex::client::CodexProvider::new(
            archon_llm::tokens::credentials_path(),
            resolution.config,
            http,
        ),
    }
    .with_context(|| format!("failed to construct direct Codex provider for {surface}"))?;
    Ok(Arc::new(provider))
}
