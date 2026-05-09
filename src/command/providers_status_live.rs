use archon_llm::providers::{AuthFlavor, ProviderDescriptor, list_compat, list_native};
use archon_llm::runtime::{ProviderHealthStatus, ProviderRuntimeStatus};
use reqwest::Url;
use serde::Serialize;

use crate::command::providers_live::ProviderLivePinger;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ProviderLiveCheck {
    pub(super) provider_id: String,
    pub(super) endpoint: Option<String>,
    pub(super) status: String,
    pub(super) detail: String,
}

impl ProviderLiveCheck {
    fn skipped(
        provider_id: impl Into<String>,
        endpoint: Option<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            endpoint,
            status: "skipped".to_string(),
            detail: detail.into(),
        }
    }

    fn from_ping(
        provider_id: impl Into<String>,
        endpoint: String,
        result: std::result::Result<(), String>,
    ) -> Self {
        match result {
            Ok(()) => Self {
                provider_id: provider_id.into(),
                endpoint: Some(endpoint),
                status: "ok".to_string(),
                detail: "endpoint reachable".to_string(),
            },
            Err(error) => Self {
                provider_id: provider_id.into(),
                endpoint: Some(endpoint),
                status: "failed".to_string(),
                detail: format!("endpoint unreachable: {error}"),
            },
        }
    }
}

pub(super) fn append_live_checks(out: &mut String, checks: &[ProviderLiveCheck]) {
    out.push_str("\nLive endpoint checks (redacted, opt-in):\n");
    out.push_str("provider             status    endpoint                 detail\n");
    out.push_str("--------------------------------------------------------------------------\n");
    for check in checks {
        out.push_str(&format!(
            "{:<20} {:<9} {:<24} {}\n",
            check.provider_id,
            check.status,
            check.endpoint.as_deref().unwrap_or("-"),
            check.detail
        ));
    }
}

pub(super) fn collect_provider_live_checks(
    statuses: &[ProviderRuntimeStatus],
    config: &archon_core::config::ArchonConfig,
    pinger: &dyn ProviderLivePinger,
) -> Vec<ProviderLiveCheck> {
    let mut descriptors = list_native();
    descriptors.extend(list_compat());
    statuses
        .iter()
        .map(|status| {
            let Some(descriptor) = descriptors
                .iter()
                .find(|descriptor| descriptor.id == status.provider_id)
            else {
                return ProviderLiveCheck::skipped(&status.provider_id, None, "provider not found");
            };
            live_check_for_status(status, descriptor, config, pinger)
        })
        .collect()
}

fn live_check_for_status(
    status: &ProviderRuntimeStatus,
    descriptor: &ProviderDescriptor,
    config: &archon_core::config::ArchonConfig,
    pinger: &dyn ProviderLivePinger,
) -> ProviderLiveCheck {
    let endpoint = match provider_live_endpoint(descriptor, config) {
        Ok(endpoint) => endpoint,
        Err(reason) => {
            return ProviderLiveCheck::skipped(&status.provider_id, None, reason);
        }
    };
    if should_skip_for_missing_credentials(status, descriptor, config) {
        return ProviderLiveCheck::skipped(
            &status.provider_id,
            Some(endpoint),
            "credentials missing",
        );
    }
    ProviderLiveCheck::from_ping(
        &status.provider_id,
        endpoint.clone(),
        pinger.ping(&endpoint),
    )
}

fn should_skip_for_missing_credentials(
    status: &ProviderRuntimeStatus,
    descriptor: &ProviderDescriptor,
    config: &archon_core::config::ArchonConfig,
) -> bool {
    if matches!(descriptor.auth_flavor, AuthFlavor::None) {
        return false;
    }
    if descriptor.id == "openai-codex" && codex_app_server_live_url(config).is_some() {
        return false;
    }
    status.health == ProviderHealthStatus::MissingCredentials
}

fn provider_live_endpoint(
    descriptor: &ProviderDescriptor,
    config: &archon_core::config::ArchonConfig,
) -> std::result::Result<String, String> {
    if descriptor.id == "openai-codex"
        && let Some(url) = codex_app_server_live_url(config)
    {
        return endpoint_from_url(&url);
    }
    endpoint_from_url(&descriptor.base_url)
}

fn codex_app_server_live_url(config: &archon_core::config::ArchonConfig) -> Option<Url> {
    let runtime = config.providers.openai_codex.runtime.trim();
    if runtime.eq_ignore_ascii_case("direct") {
        return None;
    }
    config
        .providers
        .openai_codex
        .app_server_url
        .as_deref()
        .and_then(|value| Url::parse(value).ok())
}

fn endpoint_from_url(url: &Url) -> std::result::Result<String, String> {
    match url.scheme() {
        "http" | "https" | "ws" | "wss" => {}
        other => return Err(format!("unsupported endpoint scheme: {other}")),
    }
    let host = url
        .host_str()
        .ok_or_else(|| "endpoint host missing".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "endpoint port missing".to_string())?;
    if host.contains(':') {
        Ok(format!("[{host}]:{port}"))
    } else {
        Ok(format!("{host}:{port}"))
    }
}
