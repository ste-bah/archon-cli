//! Codex runtime strategy resolution.

use anyhow::Result;
use archon_core::config::CodexProviderConfig;

use crate::runtime::provider_fallback_events::{
    record_provider_fallback_denied, record_provider_fallback_selected,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodexRuntimeDecision {
    pub(crate) selected_runtime_mode: &'static str,
    pub(crate) app_server_discovered: bool,
}

pub(crate) fn resolve_codex_runtime_strategy(
    config: &CodexProviderConfig,
    surface: &str,
) -> Result<CodexRuntimeDecision> {
    resolve_codex_runtime_strategy_with_events(config, surface, true)
}

fn resolve_codex_runtime_strategy_with_events(
    config: &CodexProviderConfig,
    surface: &str,
    emit_events: bool,
) -> Result<CodexRuntimeDecision> {
    match normalize_runtime(&config.runtime).as_str() {
        "direct" => Ok(CodexRuntimeDecision {
            selected_runtime_mode: "direct",
            app_server_discovered: false,
        }),
        "auto" => auto_strategy(config, surface, emit_events),
        "app_server" => app_server_strategy(config, surface, emit_events),
        other => Err(anyhow::anyhow!(
            "providers.openai-codex.runtime must be direct, app_server, or auto, got `{other}`"
        )),
    }
}

fn auto_strategy(
    config: &CodexProviderConfig,
    surface: &str,
    emit_events: bool,
) -> Result<CodexRuntimeDecision> {
    let discovery = crate::runtime::codex_app_server::discover_codex_app_server(config);
    if discovery.is_configured() {
        return Ok(CodexRuntimeDecision {
            selected_runtime_mode: "app_server",
            app_server_discovered: true,
        });
    }
    if !config.direct_fallback {
        return deny_direct_fallback(
            config,
            surface,
            discovery_reason(config, "codex_auto_direct_fallback_disabled"),
            emit_events,
        );
    }
    if emit_events {
        record_provider_fallback_selected(
            "openai-codex",
            "app_server",
            "direct",
            discovery.reason_code(),
            strategy_metadata(config, surface, true),
        );
    }
    Ok(CodexRuntimeDecision {
        selected_runtime_mode: "direct",
        app_server_discovered: discovery.is_configured(),
    })
}

fn app_server_strategy(
    config: &CodexProviderConfig,
    surface: &str,
    emit_events: bool,
) -> Result<CodexRuntimeDecision> {
    let discovery = crate::runtime::codex_app_server::discover_codex_app_server(config);
    if discovery.is_configured() {
        return Ok(CodexRuntimeDecision {
            selected_runtime_mode: "app_server",
            app_server_discovered: true,
        });
    }
    deny_direct_fallback(config, surface, discovery.reason_code(), emit_events)
}

fn discovery_reason(config: &CodexProviderConfig, fallback_reason: &'static str) -> &'static str {
    let discovery = crate::runtime::codex_app_server::discover_codex_app_server(config);
    if discovery.is_present() {
        discovery.reason_code()
    } else {
        fallback_reason
    }
}

fn deny_direct_fallback(
    config: &CodexProviderConfig,
    surface: &str,
    reason_code: &str,
    emit_events: bool,
) -> Result<CodexRuntimeDecision> {
    if emit_events {
        record_provider_fallback_denied(
            "openai-codex",
            "app_server",
            "direct",
            reason_code,
            strategy_metadata(config, surface, false),
        );
    }
    Err(anyhow::anyhow!("{}", strategy_error(surface, reason_code)))
}

fn strategy_error(surface: &str, reason_code: &str) -> String {
    match reason_code {
        "codex_app_server_invalid_url" => format!(
            "Codex app-server endpoint is invalid for `{surface}`; fix providers.openai-codex.app_server_url or ARCHON_CODEX_APP_SERVER_URL before using app_server mode"
        ),
        "codex_app_server_invalid_target" => format!(
            "Codex app-server transport target is invalid for `{surface}`; fix providers.openai-codex app_server_transport, app_server_url, app_server_command, or app_server_args before using app_server mode"
        ),
        "codex_auto_direct_fallback_disabled" => format!(
            "Codex app-server runtime is unavailable for `{surface}` and direct fallback is disabled; set providers.openai-codex.runtime = \"direct\" or enable direct_fallback for auto mode"
        ),
        _ => format!(
            "Codex app-server runtime is unavailable for `{surface}` and direct fallback is disabled; set providers.openai-codex.runtime = \"direct\" or enable direct_fallback for auto mode"
        ),
    }
}

fn normalize_runtime(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn strategy_metadata(
    config: &CodexProviderConfig,
    surface: &str,
    direct_fallback_used: bool,
) -> serde_json::Value {
    let discovery = crate::runtime::codex_app_server::discover_codex_app_server(config);
    let mut metadata = serde_json::json!({
        "surface": surface,
        "configured_runtime": config.runtime,
        "direct_fallback": config.direct_fallback,
        "direct_fallback_used": direct_fallback_used,
        "app_server_discovery_timeout_ms": config.app_server_discovery_timeout_ms,
    });
    if let Some(object) = metadata.as_object_mut() {
        if let Some(discovery_metadata) = discovery.metadata(config).as_object() {
            object.extend(discovery_metadata.clone());
        }
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_is_the_compatibility_default() {
        let config = CodexProviderConfig::default();

        let decision = resolve_codex_runtime_strategy_with_events(&config, "test", false).unwrap();

        assert_eq!(decision.selected_runtime_mode, "direct");
        assert!(!decision.app_server_discovered);
    }

    #[test]
    fn app_server_mode_never_silently_uses_direct() {
        let config = CodexProviderConfig {
            runtime: "app_server".into(),
            direct_fallback: true,
            ..CodexProviderConfig::default()
        };

        let error = resolve_codex_runtime_strategy_with_events(&config, "test", false)
            .unwrap_err()
            .to_string();

        assert!(error.contains("app-server runtime is unavailable"));
    }

    #[test]
    fn auto_requires_explicit_direct_fallback() {
        let config = CodexProviderConfig {
            runtime: "auto".into(),
            direct_fallback: false,
            ..CodexProviderConfig::default()
        };

        assert!(resolve_codex_runtime_strategy_with_events(&config, "test", false).is_err());
    }

    #[test]
    fn auto_can_select_direct_when_policy_allows_it() {
        let config = CodexProviderConfig {
            runtime: "auto".into(),
            direct_fallback: true,
            ..CodexProviderConfig::default()
        };

        let decision = resolve_codex_runtime_strategy_with_events(&config, "test", false).unwrap();

        assert_eq!(decision.selected_runtime_mode, "direct");
        assert!(!decision.app_server_discovered);
    }

    #[test]
    fn auto_selects_app_server_when_configured() {
        let config = CodexProviderConfig {
            runtime: "auto".into(),
            direct_fallback: true,
            app_server_url: Some("http://127.0.0.1:11434/codex".into()),
            ..CodexProviderConfig::default()
        };

        let decision = resolve_codex_runtime_strategy_with_events(&config, "test", false).unwrap();
        let metadata = strategy_metadata(&config, "test", true);

        assert_eq!(decision.selected_runtime_mode, "app_server");
        assert!(decision.app_server_discovered);
        assert_eq!(metadata["app_server_discovery"]["status"], "configured");
    }

    #[test]
    fn app_server_mode_selects_adapter_when_endpoint_configured() {
        let config = CodexProviderConfig {
            runtime: "app_server".into(),
            app_server_url: Some("http://127.0.0.1:11434/codex".into()),
            ..CodexProviderConfig::default()
        };

        let decision = resolve_codex_runtime_strategy_with_events(&config, "test", false).unwrap();

        assert_eq!(decision.selected_runtime_mode, "app_server");
        assert!(decision.app_server_discovered);
    }

    #[test]
    fn app_server_mode_reports_invalid_endpoint_before_adapter_pending() {
        let config = CodexProviderConfig {
            runtime: "app_server".into(),
            app_server_url: Some("file:///tmp/codex.sock".into()),
            ..CodexProviderConfig::default()
        };

        let error = resolve_codex_runtime_strategy_with_events(&config, "test", false)
            .unwrap_err()
            .to_string();

        assert!(error.contains("endpoint is invalid"));
    }

    #[test]
    fn app_server_mode_reports_invalid_transport_before_adapter_pending() {
        let config = CodexProviderConfig {
            runtime: "app_server".into(),
            app_server_transport: "pipe".into(),
            ..CodexProviderConfig::default()
        };

        let error = resolve_codex_runtime_strategy_with_events(&config, "test", false)
            .unwrap_err()
            .to_string();

        assert!(error.contains("transport target is invalid"));
    }
}
