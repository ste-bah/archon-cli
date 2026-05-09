//! Codex runtime strategy resolution.

use anyhow::Result;
use archon_core::config::CodexProviderConfig;

use crate::runtime::provider_fallback_events::{
    record_provider_fallback_denied, record_provider_fallback_selected,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodexRuntimeDecision {
    pub(crate) selected_runtime_mode: &'static str,
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
        }),
        "auto" if config.direct_fallback => {
            if emit_events {
                record_provider_fallback_selected(
                    "openai-codex",
                    "app_server",
                    "direct",
                    "codex_app_server_unavailable",
                    strategy_metadata(config, surface, true),
                );
            }
            Ok(CodexRuntimeDecision {
                selected_runtime_mode: "direct",
            })
        }
        "auto" => deny_direct_fallback(
            config,
            surface,
            "codex_auto_direct_fallback_disabled",
            emit_events,
        ),
        "app_server" => {
            deny_direct_fallback(config, surface, "codex_app_server_unavailable", emit_events)
        }
        other => Err(anyhow::anyhow!(
            "providers.openai-codex.runtime must be direct, app_server, or auto, got `{other}`"
        )),
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
    Err(anyhow::anyhow!(
        "Codex app-server runtime is unavailable for `{surface}` and direct fallback is disabled; set providers.openai-codex.runtime = \"direct\" or enable direct_fallback for auto mode"
    ))
}

fn normalize_runtime(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

fn strategy_metadata(
    config: &CodexProviderConfig,
    surface: &str,
    direct_fallback_used: bool,
) -> serde_json::Value {
    serde_json::json!({
        "surface": surface,
        "configured_runtime": config.runtime,
        "direct_fallback": config.direct_fallback,
        "direct_fallback_used": direct_fallback_used,
        "app_server_discovery_timeout_ms": config.app_server_discovery_timeout_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_is_the_compatibility_default() {
        let config = CodexProviderConfig::default();

        let decision = resolve_codex_runtime_strategy_with_events(&config, "test", false).unwrap();

        assert_eq!(decision.selected_runtime_mode, "direct");
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
    }
}
