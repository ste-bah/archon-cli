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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn configured_only_ignores_hostile_ambient_endpoint() {
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
        assert!(
            !serde_json::to_string(&resolved)
                .unwrap()
                .contains("credential")
        );
    }

    #[test]
    fn ordinary_policy_preserves_ambient_precedence() {
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
    }
}
