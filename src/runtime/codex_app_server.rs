//! Codex app-server discovery boundary.

use archon_core::config::CodexProviderConfig;

const APP_SERVER_URL_ENV: &str = "ARCHON_CODEX_APP_SERVER_URL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexAppServerDiscovery {
    NotConfigured,
    Configured(CodexAppServerEndpoint),
}

impl CodexAppServerDiscovery {
    pub(crate) fn is_configured(&self) -> bool {
        matches!(self, Self::Configured(_))
    }

    pub(crate) fn reason_code(&self) -> &'static str {
        match self {
            Self::Configured(_) => "codex_app_server_adapter_unimplemented",
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

pub(crate) fn discover_codex_app_server(config: &CodexProviderConfig) -> CodexAppServerDiscovery {
    discover_codex_app_server_with_env(config, std::env::var(APP_SERVER_URL_ENV).ok())
}

fn discover_codex_app_server_with_env(
    config: &CodexProviderConfig,
    env_url: Option<String>,
) -> CodexAppServerDiscovery {
    if let Some(url) = clean_url(env_url) {
        return CodexAppServerDiscovery::Configured(CodexAppServerEndpoint { url, source: "env" });
    }
    if let Some(url) = clean_url(config.app_server_url.clone()) {
        return CodexAppServerDiscovery::Configured(CodexAppServerEndpoint {
            url,
            source: "config",
        });
    }
    CodexAppServerDiscovery::NotConfigured
}

fn clean_url(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_string();
    (!value.is_empty()).then_some(value)
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
}
