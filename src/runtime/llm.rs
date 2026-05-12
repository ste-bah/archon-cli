//! Legacy LLM provider dispatch bridging `archon_core::config::LlmConfig`
//! (nested, on-disk TOML shape) → `archon_llm` providers. Preserved as a
//! thin adapter wrapper; the body was originally extracted from `main.rs`
//! by TASK-AGS-699, and TASK-AGS-710 now routes new flat-config providers
//! through `archon_llm::ActiveProvider` while keeping the 5 legacy natives
//! on their hand-rolled constructors.
//!
//! TASK-AGS-710 SPEC DEVIATION (greenlit 2026-04-13, same precedent as
//! TASK-AGS-700/703):
//!
//! TECH-AGS-PROVIDERS §1138 + the TASK-AGS-710 spec specify replacing the
//! dispatch block with `ActiveProvider::new(&cfg.llm, http.clone())`
//! called from `main.rs`, assuming `cfg.llm` is the new flat
//! `archon_llm::LlmConfig`. Four real-world mismatches force this adapter
//! layer to stay:
//!
//! 1. `cfg.llm` is `archon_core::config::LlmConfig` — a NESTED shape with
//!    per-provider subconfigs (`openai{}`, `bedrock{}`, `vertex{}`,
//!    `local{}`) matching the on-disk `config.toml` users already have.
//!    Replacing it would break every existing user config. The flat
//!    `archon_llm::LlmConfig` cannot carry `bedrock.region` /
//!    `bedrock.model_id`, `vertex.project_id` / `vertex.credentials_file`,
//!    or `local.timeout_secs` — the per-provider fields simply don't
//!    exist in the flat shape. A `From` impl would have to DROP data.
//!    Rejected.
//!
//! 2. The new `archon_llm` builder hard-errors on missing credentials
//!    (`ProviderError::MissingCredential`). The legacy `build_llm_provider`
//!    falls back to Anthropic with a `tracing::warn!`. Users running with
//!    incomplete `.openai.api_key` silently work today; hard-error is a
//!    silent behavior regression (P0). The wrapper below absorbs the
//!    fallback semantics so `ActiveProvider` can stay strict. This inverts
//!    TASK-AGS-710 Validation Criterion 7 (hard-error on missing
//!    `GROQ_API_KEY` → now: warn + Anthropic fallback, preserving legacy
//!    infallible `-> Arc<dyn LlmProvider>` signature).
//!
//! 3. `archon-llm` does NOT depend on `archon-core`, so the adapter lives
//!    here as a free function rather than as `impl From<&archon_core::...>
//!    for archon_llm::...` on the `archon_llm` side. Strictly equivalent,
//!    avoids expanding the `archon_llm` crate graph.
//!
//! 4. "Expose active handle to agent-loop state so `/model` can swap"
//!    (spec scope bullet 4) is postponed to phase-8 alongside `/model`
//!    itself — the return type stays `Arc<dyn LlmProvider>` so the 3
//!    `main.rs` call sites at L473/L2300/L3179 do not change. Changing the
//!    return type to a tuple would touch all three call sites and is out
//!    of scope for this fix.
//!
//! Net result: `main.rs` call sites unchanged, user config-file format
//! unchanged, `archon_llm` builder/`ActiveProvider` stay pure, Anthropic
//! fallback preserved on every legacy code path.

use std::sync::Arc;

use anyhow::Result;
use archon_core::config::{ArchonConfig, LlmConfig};
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::identity::{IdentityProvider, get_or_create_device_id, resolve_identity_mode};
use archon_llm::provider::LlmProvider;
use archon_llm::providers::{
    AnthropicProvider, BedrockProvider, LocalProvider, OpenAiProvider, ProviderError,
    VertexProvider,
};
use archon_llm::{ActiveProvider, LlmConfig as FlatLlmConfig};

use crate::runtime::llm_non_anthropic::build_llm_provider_without_anthropic_fallback;
use crate::runtime::provider_observer::{
    observe_llm_provider_with_profile, record_provider_fallback, runtime_mode_for_provider_name,
};

pub(crate) struct LlmProviderSelection {
    pub(crate) provider: Arc<dyn LlmProvider>,
    pub(crate) fallback_reason: Option<&'static str>,
}

/// Build the configured provider for command surfaces that can choose
/// Anthropic, Codex, or an OpenAI-compatible provider.
pub(crate) async fn build_configured_llm_provider(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    origin: &str,
) -> Result<Arc<dyn LlmProvider>> {
    if config.llm.provider == "openai-codex" {
        let (provider, runtime_mode) =
            crate::runtime::codex_provider::build_codex_provider(config, origin).await?;
        let profile_id = crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
            provider.name(),
        );
        return Ok(observe_llm_provider_with_profile(
            provider,
            runtime_mode,
            profile_id,
        ));
    }

    if config.llm.provider != "anthropic" {
        let fallback_denial_reason =
            match build_llm_provider_without_anthropic_fallback(&config.llm) {
                Ok(provider) => {
                    let selected_provider = provider.name().to_string();
                    let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
                    let profile_id =
                        crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                            &selected_provider,
                        );
                    return Ok(observe_llm_provider_with_profile(
                        provider,
                        runtime_mode,
                        profile_id,
                    ));
                }
                Err(provider_error) => {
                    tracing::warn!(
                        provider = %config.llm.provider,
                        error = %provider_error,
                        "provider construction failed before Anthropic fallback"
                    );
                    provider_construction_error_reason(&provider_error)
                }
            };

        if !anthropic_fallback_auth_available(env_vars) {
            record_anthropic_fallback_denied(&config.llm.provider, origin, fallback_denial_reason);
        }
    }

    let auth = resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    )
    .map_err(|e| anyhow::anyhow!("Authentication failed: {e}"))?;
    let identity_mode = resolve_identity_mode(&auth, false, &config.identity.as_view());
    let account_uuid = if matches!(
        identity_mode,
        archon_llm::identity::IdentityMode::Spoof { .. }
    ) {
        crate::command::utils::fetch_account_uuid(&auth).await
    } else {
        String::new()
    };
    let identity = IdentityProvider::new(
        identity_mode,
        format!("{origin}-{}", uuid::Uuid::new_v4()),
        get_or_create_device_id(),
        account_uuid,
    );
    let api_url = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| config.api.base_url.clone());
    let client = AnthropicClient::new(auth, identity, api_url);
    let selection = build_llm_provider_selection(&config.llm, &config.models, client);
    let selected_provider = selection.provider.name().to_string();
    let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
    record_provider_fallback(
        &config.llm.provider,
        &selected_provider,
        runtime_mode,
        selection
            .fallback_reason
            .unwrap_or("provider_construction_fallback"),
    );
    let profile_id = crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
        &selected_provider,
    );
    Ok(observe_llm_provider_with_profile(
        selection.provider,
        runtime_mode,
        profile_id,
    ))
}

pub(crate) fn record_anthropic_fallback_denied(
    requested_provider: &str,
    surface: &str,
    provider_error_reason: &'static str,
) {
    if requested_provider == "anthropic" {
        return;
    }
    crate::runtime::provider_fallback_events::record_provider_construction_fallback_denied(
        requested_provider,
        "anthropic",
        "anthropic_fallback_auth_unavailable",
        serde_json::json!({
            "requested_provider": requested_provider,
            "target_provider": "anthropic",
            "source": "provider_construction",
            "surface": surface,
            "provider_error_reason": provider_error_reason,
        }),
    );
}

pub(crate) fn provider_construction_error_reason(error: &anyhow::Error) -> &'static str {
    let message = error.to_string();
    if message.contains("OpenAI selected but no API key found") {
        "openai_missing_api_key"
    } else if message.contains("Bedrock selected but region/model_id missing") {
        "bedrock_missing_region_or_model"
    } else if message.contains("Vertex selected but project_id missing") {
        "vertex_missing_project_id"
    } else if message.contains("unknown provider:") {
        "openai_compatible_unknown_provider"
    } else if message.contains("missing credential: env var") {
        "openai_compatible_missing_credential"
    } else {
        "provider_construction_failed"
    }
}

fn anthropic_fallback_auth_available(env_vars: &ArchonEnvVars) -> bool {
    env_vars.anthropic_api_key.as_deref().is_some_and(has_text)
        || env_vars.archon_api_key.as_deref().is_some_and(has_text)
        || env_vars.archon_oauth_token.as_deref().is_some_and(has_text)
        || std::env::var("ANTHROPIC_AUTH_TOKEN")
            .ok()
            .as_deref()
            .is_some_and(has_text)
}

fn has_text(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Build the active LLM provider from the `[llm]` config section.
///
/// Matches on `llm_cfg.provider` to construct the appropriate provider.
/// - The 5 legacy natives (`anthropic`, `openai`, `bedrock`, `vertex`,
///   `local`) stay on their hand-rolled constructors so the nested
///   `archon_core::config::LlmConfig` sub-fields are honored.
/// - Any other provider string (`groq`, `deepseek`, `mistral`, `xai`,
///   `gemini`, `together`, `perplexity`, ...) is routed through
///   `archon_llm::ActiveProvider` with a flat `archon_llm::LlmConfig`.
///
/// Falls back to Anthropic (with a `tracing::warn!`) whenever the selected
/// provider is missing required credentials, is unrecognised, or fails to
/// construct for any other reason. The return type is intentionally
/// infallible so the three `main.rs` call sites remain untouched.
#[cfg(test)]
pub(crate) fn build_llm_provider(
    llm_cfg: &LlmConfig,
    models_cfg: &archon_core::config::ModelsConfig,
    api_client: AnthropicClient,
) -> Arc<dyn LlmProvider> {
    build_llm_provider_selection(llm_cfg, models_cfg, api_client).provider
}

pub(crate) fn build_llm_provider_selection(
    llm_cfg: &LlmConfig,
    models_cfg: &archon_core::config::ModelsConfig,
    api_client: AnthropicClient,
) -> LlmProviderSelection {
    match llm_cfg.provider.as_str() {
        "anthropic" => selected(
            AnthropicProvider::new(api_client).with_alias_map(models_cfg.anthropic.to_alias_map()),
            None,
        ),

        "openai" => {
            let inline_key = llm_cfg.openai.api_key.clone().unwrap_or_default();
            let resolved = OpenAiProvider::resolve_api_key(&inline_key);
            if resolved.is_empty() {
                tracing::warn!("OpenAI selected but no API key found; falling back to Anthropic");
                return selected(
                    AnthropicProvider::new(api_client)
                        .with_alias_map(models_cfg.anthropic.to_alias_map()),
                    Some("openai_missing_api_key"),
                );
            }
            selected(
                OpenAiProvider::new(
                    resolved,
                    llm_cfg.openai.base_url.clone(),
                    llm_cfg.openai.model.clone(),
                ),
                None,
            )
        }

        "bedrock" => {
            let region = llm_cfg.bedrock.region.clone();
            let model_id = llm_cfg.bedrock.model_id.clone();
            if region.is_empty() || model_id.is_empty() {
                tracing::warn!(
                    "Bedrock selected but region/model_id missing; falling back to Anthropic"
                );
                return selected(
                    AnthropicProvider::new(api_client)
                        .with_alias_map(models_cfg.anthropic.to_alias_map()),
                    Some("bedrock_missing_region_or_model"),
                );
            }
            selected(BedrockProvider::new(region, model_id), None)
        }

        "vertex" => {
            let project_id = llm_cfg.vertex.project_id.as_deref().unwrap_or("");
            if project_id.is_empty() {
                tracing::warn!("Vertex selected but project_id missing; falling back to Anthropic");
                return selected(
                    AnthropicProvider::new(api_client)
                        .with_alias_map(models_cfg.anthropic.to_alias_map()),
                    Some("vertex_missing_project_id"),
                );
            }
            let publisher = if llm_cfg.vertex.model.contains("claude") {
                "anthropic"
            } else {
                "google"
            };
            selected(
                VertexProvider::new(
                    project_id.to_string(),
                    llm_cfg.vertex.region.clone(),
                    llm_cfg.vertex.model.clone(),
                    publisher.to_string(),
                    llm_cfg.vertex.credentials_file.clone(),
                ),
                None,
            )
        }

        "local" => selected(
            LocalProvider::new(
                llm_cfg.local.base_url.clone(),
                llm_cfg.local.model.clone(),
                llm_cfg.local.timeout_secs,
                llm_cfg.local.pull_if_missing,
            ),
            None,
        ),

        other => {
            // Flat-config descriptor path: groq, deepseek, mistral, xai,
            // gemini, together, perplexity, etc. Route through
            // `archon_llm::ActiveProvider`
            // with a minimal flat LlmConfig; credentials come from the
            // descriptor's default env var (api_key_env override not
            // supported via nested archon_core::config::LlmConfig yet).
            let flat = FlatLlmConfig {
                provider: other.to_string(),
                model: None,
                base_url: None,
                api_key_env: None,
                retry: None,
            };
            let http = Arc::new(reqwest::Client::new());
            match ActiveProvider::new(&flat, http) {
                Ok(active) => selected(active, None),
                Err(ProviderError::MissingCredential { var }) => {
                    tracing::warn!(
                        provider = %other,
                        env_var = %var,
                        "provider credentials missing; falling back to Anthropic"
                    );
                    selected(
                        AnthropicProvider::new(api_client)
                        .with_alias_map(models_cfg.anthropic.to_alias_map()),
                        Some(flat_provider_missing_credential_reason(&var)),
                    )
                }
                Err(e) => {
                    tracing::warn!(
                        provider = %other,
                        error = %e,
                        "provider construction failed; falling back to Anthropic"
                    );
                    selected(
                        AnthropicProvider::new(api_client)
                        .with_alias_map(models_cfg.anthropic.to_alias_map()),
                        Some("openai_compatible_construction_failed"),
                    )
                }
            }
        }
    }
}

fn selected(
    provider: impl LlmProvider + 'static,
    fallback_reason: Option<&'static str>,
) -> LlmProviderSelection {
    LlmProviderSelection {
        provider: Arc::new(provider),
        fallback_reason,
    }
}

fn flat_provider_missing_credential_reason(var: &str) -> &'static str {
    if var.starts_with("unknown provider:") {
        "openai_compatible_unknown_provider"
    } else {
        "openai_compatible_missing_credential"
    }
}

#[cfg(test)]
#[path = "llm_tests.rs"]
mod tests;
