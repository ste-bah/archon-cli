//! Provider configuration adapters shared by launch identity and credential capture.
use archon_core::config::{ArchonConfig, ProviderEnvironmentConfig};
use std::collections::BTreeMap;

pub(crate) fn bindings(config: &ArchonConfig) -> BTreeMap<String, ProviderEnvironmentConfig> {
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
    for (id, binding) in include!("workflow_provider_defaults.inc.rs") {
        result.insert(id.into(), binding);
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
    result.retain(|id, _| {
        id == &config.llm.provider || config.llm.provider_environment.contains_key(id)
    });
    result
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
