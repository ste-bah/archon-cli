//! Capability-aware provider gates for user-facing command surfaces.

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_llm::providers::{ProviderCapability, capabilities_for};

pub(crate) fn ensure_active_provider_supports(
    config: &ArchonConfig,
    capability: ProviderCapability,
    surface: &str,
) -> Result<()> {
    let provider_id = active_capability_provider_id(config);
    let Some(row) = capabilities_for(provider_id) else {
        return Err(anyhow::anyhow!(
            "Provider `{}` is not in the Archon capability matrix; cannot run `{surface}`. \
             Run `archon providers capabilities` to inspect supported surfaces.",
            config.llm.provider
        ));
    };

    if row.supports(capability) {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "Provider `{}` does not support {} for `{surface}`. \
         Active support is capability-specific: Codex OAuth currently supports chat, Codex-backed TUI sessions, tool use, subagents, /btw, and provider-neutral pipelines; \
         unsupported surfaces must stay disabled until parity tests land. \
         Run `archon providers capabilities` for the source-of-truth matrix.",
        config.llm.provider,
        capability.label()
    ))
}

fn active_capability_provider_id(config: &ArchonConfig) -> &'static str {
    match config.llm.provider.as_str() {
        "openai-codex" => "openai-codex",
        "anthropic" => {
            if std::env::var("ANTHROPIC_BASE_URL").ok().is_some() || config.api.base_url.is_some() {
                "anthropic-compatible-proxy"
            } else {
                "anthropic-oauth"
            }
        }
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_allows_coding_pipeline_after_provider_neutral_adapter() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "openai-codex".into();

        ensure_active_provider_supports(
            &config,
            ProviderCapability::PipelineCoding,
            "archon pipeline code",
        )
        .expect("Codex pipeline coding support should be capability-enabled");
    }

    #[test]
    fn codex_allows_gametheory_pipeline_after_provider_neutral_adapter() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "openai-codex".into();

        ensure_active_provider_supports(
            &config,
            ProviderCapability::PipelineGametheory,
            "archon gametheory run",
        )
        .expect("Codex gametheory support should be capability-enabled");
    }

    #[test]
    fn codex_allows_subagents_after_provider_neutral_executor() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "openai-codex".into();

        ensure_active_provider_supports(
            &config,
            ProviderCapability::Subagents,
            "archon run-agent",
        )
        .expect("Codex subagent support should be capability-enabled after provider-neutral executor tests");
    }

    #[test]
    fn default_anthropic_allows_agentic_pipelines() {
        let config = ArchonConfig::default();

        ensure_active_provider_supports(
            &config,
            ProviderCapability::PipelineResearch,
            "archon pipeline research",
        )
        .expect("default Anthropic provider should support research pipeline");
    }
}
