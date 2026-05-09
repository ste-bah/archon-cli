//! Codex app-server discovery boundary.

use archon_core::config::CodexProviderConfig;
use reqwest::Url;

const APP_SERVER_URL_ENV: &str = "ARCHON_CODEX_APP_SERVER_URL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexAppServerDiscovery {
    NotConfigured,
    Invalid(CodexAppServerInvalidEndpoint),
    Configured(CodexAppServerEndpoint),
}

impl CodexAppServerDiscovery {
    pub(crate) fn is_configured(&self) -> bool {
        matches!(self, Self::Configured(_))
    }

    pub(crate) fn is_present(&self) -> bool {
        matches!(self, Self::Configured(_) | Self::Invalid(_))
    }

    pub(crate) fn status_label(&self) -> &'static str {
        match self {
            Self::Configured(_) => "configured",
            Self::Invalid(_) => "invalid",
            Self::NotConfigured => "not_configured",
        }
    }

    pub(crate) fn reason_code(&self) -> &'static str {
        match self {
            Self::Configured(_) => "codex_app_server_adapter_unimplemented",
            Self::Invalid(_) => "codex_app_server_invalid_url",
            Self::NotConfigured => "codex_app_server_unavailable",
        }
    }

    pub(crate) fn metadata(&self, config: &CodexProviderConfig) -> serde_json::Value {
        serde_json::json!({
            "app_server_discovery": match self {
                Self::Configured(endpoint) => serde_json::json!({
                    "status": "configured",
                    "source": endpoint.source,
                    "endpoint_redacted": redact_endpoint(&endpoint.url),
                }),
                Self::Invalid(endpoint) => serde_json::json!({
                    "status": "invalid",
                    "source": endpoint.source,
                    "endpoint_redacted": redact_endpoint(&endpoint.url),
                    "reason": endpoint.reason,
                }),
                Self::NotConfigured => serde_json::json!({
                    "status": "not_configured",
                }),
            },
            "fallback_model_catalog": config.app_server_model_catalog,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexAppServerEndpoint {
    pub(crate) url: String,
    pub(crate) source: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexAppServerInvalidEndpoint {
    pub(crate) url: String,
    pub(crate) source: &'static str,
    pub(crate) reason: &'static str,
}

pub(crate) fn discover_codex_app_server(config: &CodexProviderConfig) -> CodexAppServerDiscovery {
    discover_codex_app_server_with_env(config, std::env::var(APP_SERVER_URL_ENV).ok())
}

fn discover_codex_app_server_with_env(
    config: &CodexProviderConfig,
    env_url: Option<String>,
) -> CodexAppServerDiscovery {
    if let Some(url) = clean_url(env_url) {
        return discovery_for_url(url, "env");
    }
    if let Some(url) = clean_url(config.app_server_url.clone()) {
        return discovery_for_url(url, "config");
    }
    CodexAppServerDiscovery::NotConfigured
}

fn clean_url(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn discovery_for_url(url: String, source: &'static str) -> CodexAppServerDiscovery {
    match validate_endpoint_url(&url) {
        Ok(()) => CodexAppServerDiscovery::Configured(CodexAppServerEndpoint { url, source }),
        Err(reason) => CodexAppServerDiscovery::Invalid(CodexAppServerInvalidEndpoint {
            url,
            source,
            reason,
        }),
    }
}

fn validate_endpoint_url(url: &str) -> Result<(), &'static str> {
    let parsed = Url::parse(url).map_err(|_| "parse_error")?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err("unsupported_scheme"),
    }
    if parsed.host_str().is_none() {
        return Err("missing_host");
    }
    Ok(())
}

fn redact_endpoint(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}/[redacted]")
        }
        None => "[redacted]".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_prefers_env_then_config() {
        let config = CodexProviderConfig {
            app_server_url: Some("http://config.local/codex".into()),
            ..CodexProviderConfig::default()
        };

        let discovered =
            discover_codex_app_server_with_env(&config, Some("http://env.local/codex".into()));

        assert_eq!(
            discovered,
            CodexAppServerDiscovery::Configured(CodexAppServerEndpoint {
                url: "http://env.local/codex".into(),
                source: "env",
            })
        );
    }

    #[test]
    fn discovery_redacts_endpoint_metadata() {
        let config = CodexProviderConfig {
            app_server_url: Some("http://127.0.0.1:11434/private/path".into()),
            ..CodexProviderConfig::default()
        };
        let discovered = discover_codex_app_server_with_env(&config, None);
        let metadata = discovered.metadata(&config);

        assert_eq!(
            metadata["app_server_discovery"]["endpoint_redacted"],
            "http://127.0.0.1:11434/[redacted]"
        );
        assert!(metadata["fallback_model_catalog"].is_array());
    }

    #[test]
    fn discovery_rejects_invalid_endpoint_without_raw_metadata() {
        let config = CodexProviderConfig {
            app_server_url: Some("ftp://secret.example.invalid/codex?token=secret".into()),
            ..CodexProviderConfig::default()
        };
        let discovered = discover_codex_app_server_with_env(&config, None);
        let metadata = discovered.metadata(&config);

        assert_eq!(discovered.reason_code(), "codex_app_server_invalid_url");
        assert_eq!(metadata["app_server_discovery"]["status"], "invalid");
        assert_eq!(
            metadata["app_server_discovery"]["endpoint_redacted"],
            "ftp://secret.example.invalid/[redacted]"
        );
        assert_eq!(
            metadata["app_server_discovery"]["reason"],
            "unsupported_scheme"
        );
        assert!(!metadata.to_string().contains("token=secret"));
    }
}
