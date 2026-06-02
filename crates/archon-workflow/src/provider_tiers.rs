use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{WorkflowConfig, default_provider_tiers};
use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::ProviderTier;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFamily {
    Anthropic,
    OpenAiCodex,
    OpenAi,
    Gemini,
    DeepSeek,
    Ollama,
    LmStudio,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedProviderTier {
    pub tier: ProviderTier,
    pub provider_family: ProviderFamily,
    pub provider_id: String,
    pub model: String,
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderTierResolver {
    active_provider: String,
    active_model: String,
    tier_overrides: BTreeMap<ProviderTier, String>,
}

impl ProviderTierResolver {
    pub fn new(active_provider: impl Into<String>, active_model: impl Into<String>) -> Self {
        Self {
            active_provider: active_provider.into(),
            active_model: active_model.into(),
            tier_overrides: default_provider_tiers(),
        }
    }

    pub fn with_config(mut self, config: &WorkflowConfig) -> Self {
        for (tier, value) in &config.provider_tiers {
            self.tier_overrides.insert(*tier, value.clone());
        }
        self
    }

    pub fn resolve(&self, tier: ProviderTier) -> WorkflowResult<ResolvedProviderTier> {
        let target = self
            .tier_overrides
            .get(&tier)
            .map(String::as_str)
            .unwrap_or("auto");
        let provider_id = if target == "auto" {
            self.active_provider.as_str()
        } else {
            target
        };
        let family = classify_provider(provider_id)?;
        Ok(ResolvedProviderTier {
            tier,
            provider_family: family,
            provider_id: provider_id.to_string(),
            model: self.active_model.clone(),
            local: matches!(
                family,
                ProviderFamily::Ollama | ProviderFamily::LmStudio | ProviderFamily::Local
            ),
        })
    }
}

pub fn classify_provider(provider: &str) -> WorkflowResult<ProviderFamily> {
    let lower = provider.to_ascii_lowercase();
    let family = if lower.contains("openai-codex") || lower == "codex" {
        ProviderFamily::OpenAiCodex
    } else if lower.contains("anthropic") || lower.contains("claude") {
        ProviderFamily::Anthropic
    } else if lower.contains("deepseek") {
        ProviderFamily::DeepSeek
    } else if lower.contains("gemini") || lower.contains("google") {
        ProviderFamily::Gemini
    } else if lower.contains("ollama") {
        ProviderFamily::Ollama
    } else if lower.contains("lmstudio") || lower.contains("lm-studio") {
        ProviderFamily::LmStudio
    } else if lower.contains("openai") {
        ProviderFamily::OpenAi
    } else if lower.contains("local") {
        ProviderFamily::Local
    } else {
        return Err(WorkflowError::ProviderTierUnresolved(provider.to_string()));
    };
    Ok(family)
}
