//! TASK-#210 SLASH-PROVIDERS — `/providers` slash-command handler.
//!
//! Lists every LLM provider registered in the workspace (37 total =
//! 6 native + 31 OpenAI-compatible) by reading the static
//! `archon_llm::providers::{list_native, list_compat}` registries.
//! No session state is touched — both registries are readonly statics, so
//! the handler runs synchronously without populating a `CommandContext` snapshot.
//!
//! GHOST-003: 4 stub native providers (azure, cohere, copilot, minimax)
//! were removed — they returned `LlmError::Unsupported` with no real wire
//! implementations. The registry now has 6 real native entries.
//!
//! Output is a single `TuiEvent::TextDelta` carrying a two-section aligned
//! table (NATIVE then OPENAI-COMPAT), matching the `/status` / `/usage` /
//! `/extra-usage` text-delta precedent.

use anyhow::Result;

use archon_llm::providers::render_capability_table;

use crate::cli_args::ProvidersAction;

#[path = "providers_doctor.rs"]
mod providers_doctor;
#[path = "providers_registry.rs"]
mod providers_registry;

pub(crate) use crate::command::providers_slash::ProvidersHandler;
pub(crate) use providers_doctor::render_provider_doctor;
pub(crate) use providers_registry::render_provider_registry;

pub(crate) fn handle_providers(
    action: Option<ProvidersAction>,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    match action.unwrap_or(ProvidersAction::List) {
        ProvidersAction::List => print!("{}", render_provider_registry()),
        ProvidersAction::Capabilities => print!("{}", render_capability_table()),
        ProvidersAction::Status {
            provider,
            json,
            live,
        } => print!(
            "{}",
            crate::command::providers_status::render_and_persist_provider_status(
                provider.as_deref(),
                config,
                json,
                live,
            )?
        ),
        ProvidersAction::Report { provider, json } => print!(
            "{}",
            crate::command::providers_health_report::render_provider_health_report(
                provider.as_deref(),
                config,
                json,
            )?
        ),
        ProvidersAction::Limits { provider } => print!(
            "{}",
            crate::command::providers_store_cli::render_provider_limits(provider.as_deref())?
        ),
        ProvidersAction::Profiles { action } => match action {
            crate::cli_args::ProviderProfilesAction::Import => {
                print!(
                    "{}",
                    crate::command::providers_profile_import::import_provider_profiles()?
                )
            }
            crate::cli_args::ProviderProfilesAction::List { provider } => print!(
                "{}",
                crate::command::providers_store_cli::render_provider_profiles(provider.as_deref())?
            ),
            crate::cli_args::ProviderProfilesAction::Inspect { profile_id } => print!(
                "{}",
                crate::command::providers_store_cli::render_provider_profile_inspect(&profile_id)?
            ),
            crate::cli_args::ProviderProfilesAction::CooldownClear { profile_id } => print!(
                "{}",
                crate::command::providers_store_cli::clear_provider_profile_cooldown(&profile_id)?
            ),
            crate::cli_args::ProviderProfilesAction::Select {
                provider,
                auth_kinds,
                preferred,
            } => print!(
                "{}",
                crate::command::providers_store_cli::render_provider_profile_selection(
                    &provider,
                    &auth_kinds,
                    preferred.as_deref(),
                )?
            ),
        },
        ProvidersAction::Doctor { live } => print!("{}", render_provider_doctor(live)),
    }
    Ok(())
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
