use std::sync::Arc;

use crate::cli_args::Cli;
use crate::command::utils::fetch_account_uuid;
use crate::runtime::llm::{
    build_llm_provider_selection, provider_construction_error_reason,
    record_anthropic_fallback_denied,
};
use crate::runtime::llm_non_anthropic::build_llm_provider_without_anthropic_fallback;
use crate::runtime::provider_observer::{
    observe_llm_provider_with_profile, record_provider_fallback, runtime_mode_for_provider_name,
};
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::identity::{
    IdentityMode, IdentityProvider, get_or_create_device_id, resolve_identity_mode,
};

pub(super) async fn resolve_identity_and_api_client(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
) -> Result<(IdentityProvider, Option<AnthropicClient>), i32> {
    let device_id = get_or_create_device_id();
    if super::super::is_codex_session(config) || config.llm.provider != "anthropic" {
        return Ok((
            IdentityProvider::new(
                IdentityMode::Clean,
                session_id.to_string(),
                device_id,
                String::new(),
            ),
            None,
        ));
    }

    let auth = match resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
    ) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Authentication failed: {e}");
            eprintln!("Run `archon login` or set ANTHROPIC_API_KEY.");
            return Err(archon_core::print_mode::EXIT_ERROR);
        }
    };
    let identity_mode =
        resolve_identity_mode(&auth, cli.identity_spoof, &config.identity.as_view());
    let account_uuid = fetch_account_uuid(&auth).await;
    let identity = IdentityProvider::new(
        identity_mode,
        session_id.to_string(),
        device_id,
        account_uuid,
    );
    let api_url = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| config.api.base_url.clone());
    Ok((
        identity.clone(),
        Some(AnthropicClient::new(auth, identity, api_url)),
    ))
}

pub(super) async fn resolve_session_provider(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    working_dir: &std::path::Path,
    hook_registry: &Arc<archon_core::hooks::HookRegistry>,
    api_client: Option<AnthropicClient>,
) -> Result<Arc<dyn archon_llm::provider::LlmProvider>, i32> {
    let requested_provider = if super::super::is_codex_session(config) {
        "openai-codex"
    } else {
        config.llm.provider.as_str()
    };
    crate::runtime::hooks::fire_provider_resolve_hook(
        hook_registry,
        working_dir,
        session_id,
        crate::runtime::hooks::ProviderResolveHookPayload {
            hook_event: "BeforeProviderResolve",
            stage: "before_provider_resolve",
            surface: "session_agent",
            requested_provider,
            selected_provider: None,
            runtime_mode: None,
            profile_id: None,
        },
    )
    .await;

    let provider = if super::super::is_codex_session(config) {
        let (provider, runtime_mode) =
            match crate::runtime::codex_provider::build_codex_provider(config, "session_agent")
                .await
            {
                Ok(provider) => provider,
                Err(error) => {
                    eprintln!("Codex provider failed: {error}");
                    return Err(archon_core::print_mode::EXIT_ERROR);
                }
            };
        let profile_id =
            crate::runtime::provider_auth_selection::selected_provider_auth_profile_id_async(
                provider.name(),
            )
            .await;
        observe_llm_provider_with_profile(provider, runtime_mode, profile_id).await
    } else {
        let provider = match api_client {
            Some(api_client) => {
                let selection =
                    build_llm_provider_selection(&config.llm, &config.models, api_client);
                let selected_provider = selection.provider.name().to_string();
                let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
                record_provider_fallback(
                    &config.llm.provider,
                    &selected_provider,
                    runtime_mode,
                    selection
                        .fallback_reason
                        .unwrap_or("provider_construction_fallback"),
                )
                .await;
                selection.provider
            }
            None => match build_llm_provider_without_anthropic_fallback(&config.llm) {
                Ok(provider) => provider,
                Err(error) => {
                    let reason = provider_construction_error_reason(&error);
                    record_anthropic_fallback_denied(&config.llm.provider, "session_agent", reason)
                        .await;
                    eprintln!("Provider {} failed: {error}", config.llm.provider);
                    return Err(archon_core::print_mode::EXIT_ERROR);
                }
            },
        };
        let selected_provider = provider.name().to_string();
        let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
        let profile_id =
            crate::runtime::provider_auth_selection::selected_provider_auth_profile_id_async(
                &selected_provider,
            )
            .await;
        observe_llm_provider_with_profile(provider, runtime_mode, profile_id).await
    };

    let selected_provider = provider.name().to_string();
    let selected_profile_id =
        crate::runtime::provider_auth_selection::selected_provider_auth_profile_id_async(
            &selected_provider,
        )
        .await;
    crate::runtime::hooks::fire_provider_resolve_hook(
        hook_registry,
        working_dir,
        session_id,
        crate::runtime::hooks::ProviderResolveHookPayload {
            hook_event: "AfterProviderResolve",
            stage: "after_provider_resolve",
            surface: "session_agent",
            requested_provider,
            selected_provider: Some(&selected_provider),
            runtime_mode: Some(runtime_mode_for_provider_name(&selected_provider)),
            profile_id: selected_profile_id.as_deref(),
        },
    )
    .await;
    tracing::info!("LLM provider: {}", provider.name());
    Ok(provider)
}
