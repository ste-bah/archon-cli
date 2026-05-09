//! Codex app-server discovery boundary.

use archon_core::config::CodexProviderConfig;
use reqwest::Url;

const APP_SERVER_URL_ENV: &str = "ARCHON_CODEX_APP_SERVER_URL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CodexAppServerDiscovery {
    NotConfigured,
    Invalid(CodexAppServerInvalidTarget),
    Configured(CodexAppServerTarget),
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
            Self::Invalid(target) => match target.reason {
                "parse_error" | "unsupported_scheme" | "missing_host" => {
                    "codex_app_server_invalid_url"
                }
                _ => "codex_app_server_invalid_target",
            },
            Self::NotConfigured => "codex_app_server_unavailable",
        }
    }

    pub(crate) fn metadata(&self, config: &CodexProviderConfig) -> serde_json::Value {
        serde_json::json!({
            "app_server_discovery": match self {
                Self::Configured(target) => target.configured_metadata(),
                Self::Invalid(target) => target.invalid_metadata(),
                Self::NotConfigured => serde_json::json!({
                    "status": "not_configured",
                }),
            },
            "fallback_model_catalog": config.app_server_model_catalog,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexAppServerTarget {
    pub(crate) transport: String,
    pub(crate) source: &'static str,
    pub(crate) url: Option<String>,
    pub(crate) command: Option<String>,
    pub(crate) args_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexAppServerInvalidTarget {
    pub(crate) transport: String,
    pub(crate) source: &'static str,
    pub(crate) url: Option<String>,
    pub(crate) command: Option<String>,
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
        return discovery_for_url("websocket", url, "env");
    }
    discovery_for_config(config)
}

fn clean_url(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn discovery_for_config(config: &CodexProviderConfig) -> CodexAppServerDiscovery {
    let transport = normalize_transport(&config.app_server_transport);
    match transport.as_str() {
        "websocket" | "ws" => {
            if let Some(url) = clean_url(config.app_server_url.clone()) {
                discovery_for_url("websocket", url, "config")
            } else {
                CodexAppServerDiscovery::NotConfigured
            }
        }
        "stdio" => discovery_for_stdio(config),
        _ => CodexAppServerDiscovery::Invalid(CodexAppServerInvalidTarget {
            transport,
            source: "config",
            url: config.app_server_url.clone(),
            command: Some(config.app_server_command.clone()),
            reason: "unsupported_transport",
        }),
    }
}

fn discovery_for_url(
    transport: &str,
    url: String,
    source: &'static str,
) -> CodexAppServerDiscovery {
    match validate_endpoint_url(&url) {
        Ok(()) => CodexAppServerDiscovery::Configured(CodexAppServerTarget {
            transport: transport.to_string(),
            source,
            url: Some(url),
            command: None,
            args_count: 0,
        }),
        Err(reason) => CodexAppServerDiscovery::Invalid(CodexAppServerInvalidTarget {
            transport: transport.to_string(),
            source,
            url: Some(url),
            command: None,
            reason,
        }),
    }
}

fn discovery_for_stdio(config: &CodexProviderConfig) -> CodexAppServerDiscovery {
    let command = config.app_server_command.trim().to_string();
    let reason = if command.is_empty() {
        Some("missing_command")
    } else if command.contains('\0') || config.app_server_args.iter().any(|arg| arg.contains('\0'))
    {
        Some("contains_nul")
    } else {
        None
    };

    if let Some(reason) = reason {
        return CodexAppServerDiscovery::Invalid(CodexAppServerInvalidTarget {
            transport: "stdio".to_string(),
            source: "config",
            url: None,
            command: Some(command),
            reason,
        });
    }

    CodexAppServerDiscovery::Configured(CodexAppServerTarget {
        transport: "stdio".to_string(),
        source: "config",
        url: None,
        command: Some(command),
        args_count: config.app_server_args.len(),
    })
}

fn normalize_transport(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn validate_endpoint_url(url: &str) -> Result<(), &'static str> {
    let parsed = Url::parse(url).map_err(|_| "parse_error")?;
    match parsed.scheme() {
        "http" | "https" | "ws" | "wss" => {}
        _ => return Err("unsupported_scheme"),
    }
    if parsed.host_str().is_none() {
        return Err("missing_host");
    }
    Ok(())
}

impl CodexAppServerTarget {
    fn configured_metadata(&self) -> serde_json::Value {
        let mut metadata = serde_json::json!({
            "status": "configured",
            "source": self.source,
            "transport": self.transport,
        });
        add_target_metadata(
            &mut metadata,
            self.url.as_deref(),
            self.command.as_deref(),
            self.args_count,
        );
        metadata
    }
}

impl CodexAppServerInvalidTarget {
    fn invalid_metadata(&self) -> serde_json::Value {
        let mut metadata = serde_json::json!({
            "status": "invalid",
            "source": self.source,
            "transport": self.transport,
            "reason": self.reason,
        });
        add_target_metadata(
            &mut metadata,
            self.url.as_deref(),
            self.command.as_deref(),
            0,
        );
        metadata
    }
}

fn add_target_metadata(
    metadata: &mut serde_json::Value,
    url: Option<&str>,
    command: Option<&str>,
    args_count: usize,
) {
    if let Some(object) = metadata.as_object_mut() {
        if let Some(url) = url {
            object.insert(
                "endpoint_redacted".to_string(),
                serde_json::Value::String(redact_endpoint(url)),
            );
        }
        if let Some(command) = command {
            object.insert(
                "command".to_string(),
                serde_json::Value::String(redact_command(command)),
            );
            object.insert(
                "args_count".to_string(),
                serde_json::Value::Number(args_count.into()),
            );
        }
    }
}

fn redact_command(command: &str) -> String {
    std::path::Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("[redacted]")
        .to_string()
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
            CodexAppServerDiscovery::Configured(CodexAppServerTarget {
                transport: "websocket".into(),
                source: "env",
                url: Some("http://env.local/codex".into()),
                command: None,
                args_count: 0,
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

    #[test]
    fn discovery_accepts_websocket_endpoint() {
        let config = CodexProviderConfig {
            app_server_url: Some("wss://codex.example.invalid/app-server?token=secret".into()),
            ..CodexProviderConfig::default()
        };

        let discovered = discover_codex_app_server_with_env(&config, None);
        let metadata = discovered.metadata(&config);

        assert_eq!(metadata["app_server_discovery"]["transport"], "websocket");
        assert_eq!(
            metadata["app_server_discovery"]["endpoint_redacted"],
            "wss://codex.example.invalid/[redacted]"
        );
        assert!(!metadata.to_string().contains("token=secret"));
    }

    #[test]
    fn discovery_supports_stdio_transport() {
        let config = CodexProviderConfig {
            app_server_transport: "stdio".into(),
            app_server_command: "/usr/local/bin/codex".into(),
            app_server_args: vec!["app-server".into(), "--json-rpc".into()],
            ..CodexProviderConfig::default()
        };

        let discovered = discover_codex_app_server_with_env(&config, None);
        let metadata = discovered.metadata(&config);

        assert!(discovered.is_configured());
        assert_eq!(metadata["app_server_discovery"]["transport"], "stdio");
        assert_eq!(metadata["app_server_discovery"]["command"], "codex");
        assert_eq!(metadata["app_server_discovery"]["args_count"], 2);
    }

    #[test]
    fn discovery_rejects_bad_stdio_config() {
        let config = CodexProviderConfig {
            app_server_transport: "stdio".into(),
            app_server_command: " ".into(),
            ..CodexProviderConfig::default()
        };

        let discovered = discover_codex_app_server_with_env(&config, None);
        let metadata = discovered.metadata(&config);

        assert_eq!(discovered.reason_code(), "codex_app_server_invalid_target");
        assert_eq!(metadata["app_server_discovery"]["transport"], "stdio");
        assert_eq!(
            metadata["app_server_discovery"]["reason"],
            "missing_command"
        );
    }
}
