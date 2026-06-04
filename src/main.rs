#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::doc_overindented_list_items)]
#![allow(clippy::empty_line_after_doc_comments)]

mod agent_handle;
pub(crate) mod cli_args;
mod command;
mod gametheory_tool_executor;
mod main_bootstrap;
mod main_dispatch;
mod main_modes;
mod main_resume;
#[cfg(test)]
mod main_tests;
mod panic_save;
mod runtime;
pub(crate) mod session;
pub(crate) mod session_loop;
pub(crate) mod setup;
mod slash_context;

use anyhow::Result;
use clap::Parser;

use cli_args::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();
    let bootstrap = main_bootstrap::bootstrap(&cli)?;
    let config = &bootstrap.config;
    let env_vars = &bootstrap.env_vars;
    let resolved_flags = &bootstrap.resolved_flags;
    let session_id = &bootstrap.session_id;
    gametheory_tool_executor::install(config.clone(), env_vars.clone());

    // TODO(TUI-330): app::TuiEvent moves to archon_tui::events::TuiEvent
    let voice_event_rx = crate::command::tui_helpers::setup_voice_pipeline(config).await;

    if main_modes::handle_subcommand_if_present(
        &mut cli,
        config,
        env_vars,
        resolved_flags,
        &bootstrap.working_dir_for_config,
    )
    .await?
    {
        return Ok(());
    }

    if main_modes::handle_headless_if_requested(&cli, config, env_vars, resolved_flags, session_id)
        .await?
    {
        return Ok(());
    }

    if main_modes::handle_catalog_modes_if_requested(&cli, config)? {
        return Ok(());
    }
    if main_resume::handle_resume_list_if_requested(&cli, config).await? {
        return Ok(());
    }

    let mut resume_messages = main_resume::load_explicit_resume_messages(&cli, config)?;
    main_resume::maybe_continue_session(&cli, config, &mut resume_messages);
    main_resume::maybe_auto_resume(&cli, config, &mut resume_messages);
    if main_modes::handle_session_management_if_requested(&cli, config)? {
        return Ok(());
    }
    if main_modes::handle_background_if_requested(&cli)? {
        return Ok(());
    }
    if main_modes::handle_print_mode_if_requested(
        &cli,
        config,
        env_vars,
        resolved_flags,
        session_id,
    )
    .await?
    {
        return Ok(());
    }
    main_modes::ensure_interactive_tty()?;

    crate::session::run_interactive_session(
        config,
        session_id,
        &cli,
        env_vars,
        resume_messages,
        resolved_flags,
        voice_event_rx,
    )
    .await
}

fn resolve_json_schema(cli: &Cli) -> Result<Option<String>> {
    if let Some(schema) = &cli.json_schema {
        return Ok(Some(schema.clone()));
    }
    let Some(path) = &cli.json_schema_path else {
        return Ok(None);
    };
    let schema = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read JSON schema from {}: {e}", path.display()))?;
    Ok(Some(schema))
}
