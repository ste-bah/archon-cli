//! Codex model selection helpers for surfaces that inherit global defaults.

use archon_core::config::ArchonConfig;

pub(crate) fn codex_model_for_anthropic_default(config: &ArchonConfig) -> Option<String> {
    codex_model_for_anthropic_model(config, &config.api.default_model)
}

pub(crate) fn codex_model_for_anthropic_model(
    config: &ArchonConfig,
    model: &str,
) -> Option<String> {
    let requested = model.trim();
    let lower = requested.to_ascii_lowercase();
    if !lower.starts_with("claude") {
        return None;
    }

    let aliases = config.models.openai_codex.to_alias_map();
    if lower.contains("haiku") {
        Some(aliases.haiku)
    } else if lower.contains("opus") {
        Some(aliases.opus)
    } else {
        Some(aliases.sonnet)
    }
}
