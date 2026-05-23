use archon_core::config::ArchonConfig;

pub(crate) fn active_session_model(config: &ArchonConfig) -> String {
    if super::is_codex_session(config)
        && let Some(model) = crate::runtime::codex_model::codex_model_for_anthropic_default(config)
    {
        return model;
    }

    if config.llm.provider == "anthropic" {
        return env_model("ANTHROPIC_MODEL").unwrap_or_else(|| config.api.default_model.clone());
    }

    let api_model = config.api.default_model.trim();
    if !is_legacy_anthropic_model(api_model) {
        return config.api.default_model.clone();
    }

    configured_provider_model(config).unwrap_or_else(|| config.api.default_model.clone())
}

fn env_model(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(non_empty)
}

fn configured_provider_model(config: &ArchonConfig) -> Option<String> {
    let model = match config.llm.provider.as_str() {
        "openai" => config.llm.openai.model.clone(),
        "bedrock" => config.llm.bedrock.model_id.clone(),
        "vertex" => config.llm.vertex.model.clone(),
        "local" => config.llm.local.model.clone(),
        other => descriptor_default_model(other)?,
    };
    non_empty(model)
}

fn descriptor_default_model(provider: &str) -> Option<String> {
    let flat = archon_llm::LlmConfig {
        provider: provider.to_string(),
        model: None,
        base_url: None,
        api_key_env: None,
        retry: None,
    };
    flat.resolve_descriptor()
        .ok()
        .map(|descriptor| descriptor.default_model.clone())
}

fn non_empty(model: String) -> Option<String> {
    if model.trim().is_empty() {
        None
    } else {
        Some(model)
    }
}

fn is_legacy_anthropic_model(model: &str) -> bool {
    let lower = model.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower == "opus"
        || lower == "sonnet"
        || lower == "haiku"
        || lower.starts_with("claude-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_uses_registry_default_when_api_model_is_legacy_anthropic() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "deepseek".into();
        config.api.default_model = "claude-sonnet-4-6".into();

        assert_eq!(active_session_model(&config), "deepseek-v4-flash");
    }

    #[test]
    fn deepseek_preserves_explicit_api_model_override() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "deepseek".into();
        config.api.default_model = "deepseek-v4-pro[1m]".into();

        assert_eq!(active_session_model(&config), "deepseek-v4-pro[1m]");
    }

    #[test]
    fn openai_uses_nested_model_when_api_model_is_legacy_anthropic() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "openai".into();
        config.api.default_model = "claude-sonnet-4-6".into();
        config.llm.openai.model = "gpt-4.1".into();

        assert_eq!(active_session_model(&config), "gpt-4.1");
    }

    #[test]
    fn active_session_model_uses_configured_codex_default_when_claude_default_would_leak() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "openai-codex".into();
        config.api.default_model = "claude-sonnet-4-6".into();
        config.models.openai_codex.default = "gpt-codex-default".into();

        assert_eq!(active_session_model(&config), "gpt-codex-default");
    }

    #[test]
    fn active_session_model_preserves_anthropic_default() {
        let config = ArchonConfig::default();

        assert_eq!(active_session_model(&config), config.api.default_model);
    }

    #[test]
    fn anthropic_session_honors_anthropic_model_env_override() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "anthropic".into();
        unsafe {
            std::env::set_var("ANTHROPIC_MODEL", "deepseek-v4-pro[1m]");
        }
        let model = active_session_model(&config);
        unsafe {
            std::env::remove_var("ANTHROPIC_MODEL");
        }

        assert_eq!(model, "deepseek-v4-pro[1m]");
    }
}
