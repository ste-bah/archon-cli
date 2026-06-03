use std::collections::BTreeMap;

use archon_llm::provider::{DataFlowClassification, LlmRequest, ProviderFeature, ProviderRegistry};
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
    pub data_flow: String,
    pub features: Vec<String>,
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
            data_flow: fallback_data_flow(family).to_string(),
            features: fallback_features(family),
        })
    }

    pub fn resolve_with_registry(
        &self,
        tier: ProviderTier,
        registry: &ProviderRegistry,
    ) -> WorkflowResult<ResolvedProviderTier> {
        let provider = registry
            .active(&self.active_provider)
            .map_err(|err| WorkflowError::ProviderTierUnresolved(err.to_string()))?;
        let hint = self
            .tier_overrides
            .get(&tier)
            .map(String::as_str)
            .unwrap_or("auto");
        let requested = if hint == "auto" {
            tier_alias(tier)
        } else {
            hint
        };
        let mut request = LlmRequest {
            model: requested.to_string(),
            ..LlmRequest::default()
        };
        provider.resolve_request_model(&mut request);
        let flow = provider.data_flow_classification();
        enforce_tier_requirements(
            tier,
            provider.supports_feature(ProviderFeature::Vision),
            flow,
        )?;
        let provider_id = provider.name().to_string();
        Ok(ResolvedProviderTier {
            tier,
            provider_family: classify_provider(&provider_id)?,
            provider_id,
            model: request.model,
            local: flow == DataFlowClassification::Local,
            data_flow: data_flow_label(flow).to_string(),
            features: supported_features(provider),
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

fn enforce_tier_requirements(
    tier: ProviderTier,
    vision: bool,
    flow: DataFlowClassification,
) -> WorkflowResult<()> {
    if tier == ProviderTier::Vision && !vision {
        return Err(WorkflowError::ProviderTierUnresolved(
            "vision tier requires ProviderFeature::Vision".into(),
        ));
    }
    if tier == ProviderTier::Local && flow != DataFlowClassification::Local {
        return Err(WorkflowError::ProviderTierUnresolved(
            "local tier requires local data flow".into(),
        ));
    }
    Ok(())
}

fn supported_features(provider: &dyn archon_llm::provider::LlmProvider) -> Vec<String> {
    [
        (ProviderFeature::Thinking, "thinking"),
        (ProviderFeature::ToolUse, "tool_use"),
        (ProviderFeature::PromptCaching, "prompt_caching"),
        (ProviderFeature::Vision, "vision"),
        (ProviderFeature::SystemPrompt, "system_prompt"),
        (ProviderFeature::Streaming, "streaming"),
    ]
    .into_iter()
    .filter_map(|(feature, label)| provider.supports_feature(feature).then_some(label.into()))
    .collect()
}

fn tier_alias(tier: ProviderTier) -> &'static str {
    match tier {
        ProviderTier::Cheap | ProviderTier::Local => "haiku",
        ProviderTier::Critic | ProviderTier::Reducer => "opus",
        ProviderTier::Planner
        | ProviderTier::Researcher
        | ProviderTier::Coder
        | ProviderTier::Vision => "sonnet",
    }
}

fn data_flow_label(flow: DataFlowClassification) -> &'static str {
    match flow {
        DataFlowClassification::Local => "local",
        DataFlowClassification::UserOperated => "user_operated",
        DataFlowClassification::Cloud => "cloud",
    }
}

fn fallback_data_flow(family: ProviderFamily) -> &'static str {
    match family {
        ProviderFamily::Ollama | ProviderFamily::LmStudio | ProviderFamily::Local => "local",
        _ => "cloud",
    }
}

fn fallback_features(family: ProviderFamily) -> Vec<String> {
    let mut features = vec!["tool_use".to_string(), "streaming".to_string()];
    if !matches!(family, ProviderFamily::Ollama | ProviderFamily::LmStudio) {
        features.push("vision".to_string());
    }
    features
}
