//! Provider configuration adapters shared by launch identity and credential capture.
use archon_core::config::{ArchonConfig, ProviderEnvironmentConfig};
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderDefault {
    id: String,
    base_url_config: Option<String>,
    environment: ProviderEnvironmentConfig,
}

pub(crate) fn all_bindings(config: &ArchonConfig) -> BTreeMap<String, ProviderEnvironmentConfig> {
    let mut result = BTreeMap::new();
    // Registry descriptors own standard key names. Explicit bindings override them.
    for descriptor in archon_llm::providers::registry::list_compat() {
        result.insert(
            descriptor.id.clone(),
            ProviderEnvironmentConfig {
                base_url: Some(descriptor.base_url.to_string()),
                api_key_env: Some(descriptor.env_key_var.clone()).filter(|v| !v.is_empty()),
                ..Default::default()
            },
        );
    }
    let defaults: Vec<ProviderDefault> =
        serde_json::from_str(include_str!("workflow_provider_defaults.json"))
            .expect("invalid shipped provider environment data");
    let config_value = serde_json::to_value(config).expect("configuration must serialize");
    for entry in defaults {
        let mut binding = entry.environment;
        if let Some(path) = entry.base_url_config {
            let value = config_value
                .pointer(&path)
                .expect("shipped provider endpoint path must exist in configuration");
            assert!(
                value.is_null() || value.is_string(),
                "invalid provider endpoint value at {path}"
            );
            binding.base_url = value.as_str().map(str::to_string);
        }
        result.insert(entry.id, binding);
    }
    for (id, configured) in &config.llm.provider_environment {
        let binding = result.entry(id.clone()).or_default();
        if configured.base_url.is_some() {
            binding.base_url = configured.base_url.clone();
        }
        if configured.base_url_env.is_some() {
            binding.base_url_env = configured.base_url_env.clone();
        }
        if configured.api_key_env.is_some() {
            binding.api_key_env = configured.api_key_env.clone();
        }
        binding
            .credential_env
            .extend(configured.credential_env.clone());
        binding.credential_env.sort();
        binding.credential_env.dedup();
    }
    result
}

pub(crate) fn bindings(config: &ArchonConfig) -> BTreeMap<String, ProviderEnvironmentConfig> {
    let mut result = all_bindings(config);
    result.retain(|id, _| {
        id == &config.llm.provider || config.llm.provider_environment.contains_key(id)
    });
    result
}

pub(crate) fn runtime_config(
    config: &ArchonConfig,
    policy: super::ProviderEndpointPolicy,
) -> archon_core::config::LlmConfig {
    let mut llm = config.llm.clone();
    if let Some(mut binding) =
        crate::command::workflow_provider_route::bindings(config).remove(&llm.provider)
    {
        let ambient = binding
            .base_url_env
            .as_deref()
            .and_then(|name| std::env::var(name).ok());
        binding.base_url = archon_core::config::model_policy::endpoint(
            binding.base_url.as_deref(),
            ambient.as_deref(),
            policy
                == crate::command::workflow_provider_route::ProviderEndpointPolicy::AmbientAllowed,
        );
        if llm.provider == "openai" {
            llm.openai.base_url = binding.base_url.clone();
        }
        if llm.provider == "local" {
            if let Some(endpoint) = &binding.base_url {
                llm.local.base_url = endpoint.clone();
            }
        }
        llm.provider_environment
            .insert(llm.provider.clone(), binding);
    }
    llm
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_config_preserves_the_configured_key_for_the_base_resolver() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "openai".into();
        config.llm.openai.api_key = Some("configured-fixture-key".into());
        let runtime = runtime_config(
            &config,
            super::super::ProviderEndpointPolicy::ConfiguredOnly,
        );
        assert_eq!(runtime.openai.api_key, config.llm.openai.api_key);
    }

    #[test]
    fn fallback_keeps_the_same_endpoint_registry_as_direct_selection() {
        let mut config = ArchonConfig::default();
        let direct = all_bindings(&config)["anthropic"].clone();
        config.llm.provider = "openai".into();
        let fallback = all_bindings(&config)["anthropic"].clone();
        assert_eq!(direct.base_url_env, fallback.base_url_env);
        assert_eq!(fallback.base_url_env.as_deref(), Some("ANTHROPIC_BASE_URL"));
        config.api.base_url = Some("https://proxy.invalid/messages".into());
        assert_eq!(
            all_bindings(&config)["anthropic"].base_url,
            config.api.base_url
        );
    }

    #[test]
    fn registry_and_explicit_environment_names_are_preserved() {
        let mut config = ArchonConfig::default();
        config.llm.provider = "groq".into();
        assert_eq!(
            bindings(&config)["groq"].api_key_env.as_deref(),
            Some("GROQ_API_KEY")
        );
        config.llm.provider_environment.insert(
            "private-service".into(),
            ProviderEnvironmentConfig {
                base_url: Some("https://private.invalid/v1".into()),
                base_url_env: Some("PRIVATE_URL".into()),
                api_key_env: Some("PRIVATE_KEY".into()),
                credential_env: vec!["PRIVATE_TOKEN".into()],
            },
        );
        let values = bindings(&config);
        assert_eq!(values.len(), 2);
        assert_eq!(
            values["private-service"].base_url_env.as_deref(),
            Some("PRIVATE_URL")
        );
        assert_eq!(values["private-service"].credential_env, ["PRIVATE_TOKEN"]);
    }
}
