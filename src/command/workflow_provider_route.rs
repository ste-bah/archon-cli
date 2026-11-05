//! Trusted provider-route policy for fixed decomposition.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderEndpointPolicy {
    AmbientAllowed,
    ConfiguredOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TrustedProviderRouteSnapshot {
    pub(crate) origin: String,
    pub(crate) endpoint: Option<String>,
    pub(crate) endpoint_digest: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) providers: std::collections::BTreeMap<String, serde_json::Value>,
}

#[path = "workflow_provider_bindings.rs"]
mod provider_bindings;
pub(crate) use provider_bindings::{bindings, runtime_config};

/// Snapshot every configured provider's endpoint and environment binding names.
pub(crate) fn resolve_provider_routes(
    config: &archon_core::config::ArchonConfig,
    policy: ProviderEndpointPolicy,
) -> TrustedProviderRouteSnapshot {
    let mut route = resolve_anthropic_route(config.api.base_url.as_deref(), policy);
    route.origin = config.llm.provider.clone();
    route.endpoint = None;
    route.endpoint_digest = None;
    route.providers = bindings(config)
        .into_iter()
        .map(|(id, binding)| {
            let ambient = binding
                .base_url_env
                .as_deref()
                .and_then(|name| std::env::var(name).ok())
                .filter(|value| !value.trim().is_empty());
            let endpoint = archon_core::config::model_policy::endpoint(
                binding.base_url.as_deref(),
                ambient.as_deref(),
                policy == ProviderEndpointPolicy::AmbientAllowed,
            );
            let digest =
                |value: &str| archon_workflow::task_set_contract::content_digest(value.as_bytes());
            (
                id,
                serde_json::json!({
                        "endpoint_digest": endpoint.as_deref().map(digest),
                        "ambient_endpoint_digest": ambient.as_deref().map(digest),
                        "base_url_env": binding.base_url_env,
                        "api_key_env": binding.api_key_env,
                        "credential_env": binding.credential_env,
                "role_models": config.llm.role_models,
                "tier_aliases": config.llm.tier_aliases,
                    }),
            )
        })
        .collect();
    route
}

pub(crate) fn resolve_anthropic_route(
    configured_endpoint: Option<&str>,
    policy: ProviderEndpointPolicy,
) -> TrustedProviderRouteSnapshot {
    let (origin, endpoint) = match policy {
        ProviderEndpointPolicy::AmbientAllowed => match std::env::var("ANTHROPIC_BASE_URL") {
            Ok(value) if !value.trim().is_empty() => ("ambient_env", Some(value)),
            _ => (
                "configured",
                configured_endpoint
                    .map(str::to_string)
                    .filter(|v| !v.trim().is_empty()),
            ),
        },
        ProviderEndpointPolicy::ConfiguredOnly => (
            "trusted_config",
            configured_endpoint
                .map(str::to_string)
                .filter(|v| !v.trim().is_empty()),
        ),
    };
    let endpoint_digest = endpoint
        .as_deref()
        .map(|value| archon_workflow::task_set_contract::content_digest(value.as_bytes()));
    TrustedProviderRouteSnapshot {
        origin: origin.to_string(),
        endpoint,
        endpoint_digest,
        providers: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn configured_only_ignores_hostile_ambient_endpoint() {
        if crate::test_environment::isolated() {
            return;
        }
        let _guard = ENV.lock().unwrap();
        unsafe { std::env::set_var("ANTHROPIC_BASE_URL", "https://hostile.invalid/v1") };
        let resolved = resolve_anthropic_route(
            Some("https://trusted.example/v1/messages"),
            ProviderEndpointPolicy::ConfiguredOnly,
        );
        unsafe { std::env::remove_var("ANTHROPIC_BASE_URL") };

        assert_eq!(resolved.origin, "trusted_config");
        assert_eq!(
            resolved.endpoint.as_deref(),
            Some("https://trusted.example/v1/messages")
        );
        assert_eq!(
            resolved.endpoint_digest,
            Some(archon_workflow::task_set_contract::content_digest(
                b"https://trusted.example/v1/messages"
            ))
        );
        assert!(
            !serde_json::to_string(&resolved)
                .unwrap()
                .contains("credential")
        );
    }

    #[test]
    fn ordinary_policy_preserves_ambient_precedence() {
        if crate::test_environment::isolated() {
            return;
        }
        let _guard = ENV.lock().unwrap();
        unsafe { std::env::set_var("ANTHROPIC_BASE_URL", "https://ambient.example/v1") };
        let resolved = resolve_anthropic_route(
            Some("https://configured.example/v1"),
            ProviderEndpointPolicy::AmbientAllowed,
        );
        unsafe { std::env::remove_var("ANTHROPIC_BASE_URL") };

        assert_eq!(resolved.origin, "ambient_env");
        assert_eq!(
            resolved.endpoint.as_deref(),
            Some("https://ambient.example/v1")
        );
        assert_eq!(
            resolved.endpoint_digest,
            Some(archon_workflow::task_set_contract::content_digest(
                b"https://ambient.example/v1"
            ))
        );
    }
}

#[cfg(test)]
#[test]
fn snapshots_all_configured_routes_without_credentials() {
    let mut config = archon_core::config::ArchonConfig::default();
    config.llm.provider = "groq".into();
    config.llm.provider_environment.insert(
        "private-service".into(),
        archon_core::config::ProviderEnvironmentConfig {
            base_url: Some("https://private.invalid/v1".into()),
            base_url_env: Some("PRIVATE_ROUTE_URL".into()),
            api_key_env: Some("PRIVATE_ROUTE_KEY".into()),
            ..Default::default()
        },
    );
    let before = resolve_provider_routes(&config, ProviderEndpointPolicy::ConfiguredOnly);
    assert_eq!(before.providers.len(), 2);
    assert_eq!(before.providers["groq"]["api_key_env"], "GROQ_API_KEY");
    config
        .llm
        .provider_environment
        .get_mut("private-service")
        .unwrap()
        .base_url = Some("https://changed.invalid/v1".into());
    let after = resolve_provider_routes(&config, ProviderEndpointPolicy::ConfiguredOnly);
    assert_ne!(before, after);
    let serialized = serde_json::to_string(&after).unwrap();
    assert!(!serialized.contains("https://changed.invalid"));
    assert!(serialized.contains("PRIVATE_ROUTE_KEY"));
}
