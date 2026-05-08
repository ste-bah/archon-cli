use std::collections::HashSet;

use archon_llm::providers::{AuthFlavor, ProviderDescriptor, list_compat, list_native};
use archon_llm::runtime::{ProviderHealthStatus, ProviderIdentityStatus, ProviderRuntimeStatus};

pub(crate) fn render_provider_status(provider_filter: Option<&str>) -> String {
    render_provider_status_with_env(provider_filter, &ProviderStatusEnv::detect())
}

fn render_provider_status_with_env(
    provider_filter: Option<&str>,
    env: &ProviderStatusEnv,
) -> String {
    let mut descriptors = list_native();
    descriptors.extend(list_compat());
    descriptors.sort_by(|a, b| a.id.cmp(&b.id));

    let statuses: Vec<ProviderRuntimeStatus> = descriptors
        .into_iter()
        .filter(|descriptor| provider_filter.map_or(true, |filter| descriptor.id == filter))
        .map(|descriptor| status_from_descriptor(descriptor, env))
        .collect();

    let mut out = String::new();
    out.push_str("Provider runtime status (local configuration)\n\n");
    if statuses.is_empty() {
        out.push_str("No provider matched the requested filter.\n");
        return out;
    }
    out.push_str("provider             health               mode        identity    model\n");
    out.push_str("--------------------------------------------------------------------------\n");
    for status in statuses {
        out.push_str(&format!(
            "{:<20} {:<20} {:<11} {:<11} {}\n",
            status.provider_id,
            health_label(status.health),
            status.runtime_mode,
            identity_label(status.identity_status),
            status.model_id.as_deref().unwrap_or("n/a"),
        ));
    }
    out.push_str(
        "\nThis status is local and redacted; use `archon providers doctor --live` for opt-in endpoint checks.\n",
    );
    out
}

fn status_from_descriptor(
    descriptor: &ProviderDescriptor,
    env: &ProviderStatusEnv,
) -> ProviderRuntimeStatus {
    let mut status = ProviderRuntimeStatus::new(descriptor.id.clone(), runtime_mode(descriptor))
        .with_display_name(descriptor.display_name.clone())
        .with_model(descriptor.default_model.clone())
        .with_identity_status(identity_status(descriptor, env));
    let health = if credentials_present(descriptor, env) {
        ProviderHealthStatus::Unknown
    } else {
        ProviderHealthStatus::MissingCredentials
    };
    status = status.with_health(health);
    status
}

fn runtime_mode(descriptor: &ProviderDescriptor) -> &'static str {
    if descriptor.id == "openai-codex" {
        "auto"
    } else if matches!(descriptor.auth_flavor, AuthFlavor::None) {
        "local"
    } else {
        "direct"
    }
}

fn identity_status(
    descriptor: &ProviderDescriptor,
    env: &ProviderStatusEnv,
) -> ProviderIdentityStatus {
    match descriptor.id.as_str() {
        "anthropic" if env.anthropic_oauth || env.anthropic_bearer_env => {
            ProviderIdentityStatus::Spoof
        }
        "anthropic" => ProviderIdentityStatus::Clean,
        "openai-codex" if env.codex_oauth => ProviderIdentityStatus::AppServer,
        "openai-codex" => ProviderIdentityStatus::Custom,
        _ if matches!(descriptor.auth_flavor, AuthFlavor::None) => {
            ProviderIdentityStatus::NotApplicable
        }
        _ => ProviderIdentityStatus::Clean,
    }
}

fn credentials_present(descriptor: &ProviderDescriptor, env: &ProviderStatusEnv) -> bool {
    match descriptor.id.as_str() {
        "anthropic" => env.anthropic_oauth || env.has_env_var(&descriptor.env_key_var),
        "openai-codex" => env.codex_oauth,
        _ if matches!(descriptor.auth_flavor, AuthFlavor::None) => true,
        _ => env.has_env_var(&descriptor.env_key_var),
    }
}

fn health_label(health: ProviderHealthStatus) -> &'static str {
    match health {
        ProviderHealthStatus::Healthy => "healthy",
        ProviderHealthStatus::Degraded => "degraded",
        ProviderHealthStatus::Unavailable => "unavailable",
        ProviderHealthStatus::MissingCredentials => "missing-credentials",
        ProviderHealthStatus::Unknown => "unknown-local",
    }
}

fn identity_label(identity: ProviderIdentityStatus) -> &'static str {
    match identity {
        ProviderIdentityStatus::Clean => "clean",
        ProviderIdentityStatus::Spoof => "spoof",
        ProviderIdentityStatus::Custom => "custom",
        ProviderIdentityStatus::AppServer => "app-server",
        ProviderIdentityStatus::NotApplicable => "n/a",
    }
}

#[derive(Debug, Default)]
struct ProviderStatusEnv {
    env_vars: HashSet<String>,
    anthropic_oauth: bool,
    anthropic_bearer_env: bool,
    codex_oauth: bool,
}

impl ProviderStatusEnv {
    fn detect() -> Self {
        let mut env = Self {
            env_vars: std::env::vars()
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, _)| key)
                .collect(),
            ..Self::default()
        };
        env.anthropic_bearer_env = std::env::var("ANTHROPIC_API_KEY")
            .map(|value| value.starts_with("sk-ant-oat"))
            .unwrap_or(false);
        let path = archon_llm::tokens::credentials_path();
        if let Ok(json) = std::fs::read_to_string(path) {
            env.anthropic_oauth = archon_llm::auth::parse_credentials_json(&json).is_ok();
            env.codex_oauth = archon_llm::auth::parse_codex_credentials_json(&json).is_ok();
        }
        env
    }

    fn has_env_var(&self, name: &str) -> bool {
        !name.is_empty() && self.env_vars.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(vars: &[&str]) -> ProviderStatusEnv {
        ProviderStatusEnv {
            env_vars: vars.iter().map(|name| name.to_string()).collect(),
            ..ProviderStatusEnv::default()
        }
    }

    #[test]
    fn status_lists_local_provider_without_credentials() {
        let body = render_provider_status_with_env(Some("ollama"), &ProviderStatusEnv::default());

        assert!(body.contains("ollama"));
        assert!(body.contains("unknown-local"));
        assert!(body.contains("local"));
        assert!(body.contains("n/a"));
    }

    #[test]
    fn status_marks_missing_credentials_for_remote_provider() {
        let body = render_provider_status_with_env(Some("openai"), &ProviderStatusEnv::default());

        assert!(body.contains("openai"));
        assert!(body.contains("missing-credentials"));
    }

    #[test]
    fn status_marks_configured_env_provider_as_unknown_local() {
        let body = render_provider_status_with_env(Some("openai"), &env_with(&["OPENAI_API_KEY"]));

        assert!(body.contains("openai"));
        assert!(body.contains("unknown-local"));
    }

    #[test]
    fn status_shows_anthropic_spoof_for_oauth_profile() {
        let env = ProviderStatusEnv {
            anthropic_oauth: true,
            ..ProviderStatusEnv::default()
        };
        let body = render_provider_status_with_env(Some("anthropic"), &env);

        assert!(body.contains("anthropic"));
        assert!(body.contains("spoof"));
    }

    #[test]
    fn status_reports_empty_filter_result() {
        let body = render_provider_status_with_env(Some("missing-provider"), &env_with(&[]));

        assert!(body.contains("No provider matched"));
    }
}
