#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::doc_overindented_list_items)]
#![allow(clippy::empty_line_after_doc_comments)]
// Session/workflow entry points thread wide context through plain arguments;
// restructuring them is the threading-model follow-up recorded in the rescue
// phase-5 report, a working document held outside the repository.
#![allow(clippy::too_many_arguments)]

#[cfg(test)]
mod test_environment;

mod agent_handle;
pub(crate) mod cli_args;
mod command;
mod gametheory_tool_executor;
mod main_bootstrap;
#[cfg(test)]
#[path = "main_bootstrap_tests.rs"]
mod main_bootstrap_tests;
mod main_dispatch;
mod main_modes;
mod main_resume;
mod main_startup;
#[cfg(test)]
mod main_tests;
#[cfg(test)]
mod main_voice_tests;
mod panic_save;
mod runtime;
pub(crate) mod session;
pub(crate) mod session_loop;
pub(crate) mod setup;
mod slash_context;

use anyhow::Result;
use clap::Parser;

use cli_args::Cli;

fn main() -> Result<()> {
    // SAFETY: no runtime or application threads exist yet. A limit that
    // cannot be lowered is reported once logging runs; it never stops startup.
    #[cfg(unix)]
    unsafe {
        archon_shell::process_nofile::initialize();
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(main_outcome())
}

async fn main_outcome() -> Result<()> {
    let outcome = run().await;
    // Issue 338: a command that read nothing over a task-set publish no read
    // could settle ends as the host-command contract's unsettled publish, so
    // a parent that ran it as a child pauses its run instead of failing it.
    if let Err(error) = &outcome {
        command::workflow_host_command_operational::exit_if_unsettled_publish(error);
    }
    outcome
}

async fn run() -> Result<()> {
    // Issue 336: a check-source repin settles an interrupted task-set publish
    // before it writes, by the host's recovery.
    command::workflow_task_set::register_publish_settle();
    if command::acceptance_scratch_guardian::entry().await? {
        return Ok(());
    }
    let cli = Cli::parse();
    let bootstrap = main_bootstrap::bootstrap(&cli)?;
    let config = &bootstrap.config;
    let env_vars = &bootstrap.env_vars;
    let resolved_flags = &bootstrap.resolved_flags;
    let session_id = &bootstrap.session_id;
    gametheory_tool_executor::install(config.clone(), env_vars.clone());

    run_interactive_after_dispatch(
        config,
        dispatch_modes(
            cli,
            config,
            env_vars,
            resolved_flags,
            session_id,
            &bootstrap.working_dir_for_config,
        ),
        |(cli, resume_messages), voice_event_rx| async move {
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
        },
    )
    .await
}

/// Keep the production voice callback shared with the interactive wiring tests.
/// Dispatch and the session consumer may be controlled without replacing setup.
async fn run_interactive_after_dispatch<S, IF>(
    config: &archon_core::config::ArchonConfig,
    dispatch: impl std::future::Future<Output = Result<Option<S>>>,
    interactive: impl FnOnce(S, Option<tokio::sync::mpsc::Receiver<archon_tui::app::TuiEvent>>) -> IF,
) -> Result<()>
where
    IF: std::future::Future<Output = Result<()>>,
{
    main_startup::run(
        dispatch,
        || crate::command::tui_helpers::setup_voice_pipeline(config),
        interactive,
    )
    .await
}

type InteractiveInput = (Cli, Option<Vec<serde_json::Value>>);

/// Dispatch every noninteractive mode and validate the terminal before any
/// interactive resources are started.
async fn dispatch_modes(
    mut cli: Cli,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    session_id: &str,
    working_dir_for_config: &std::path::PathBuf,
) -> Result<Option<InteractiveInput>> {
    if main_modes::handle_subcommand_if_present(
        &mut cli,
        config,
        env_vars,
        resolved_flags,
        working_dir_for_config,
    )
    .await?
    {
        return Ok(None);
    }

    if main_modes::handle_headless_if_requested(&cli, config, env_vars, resolved_flags, session_id)
        .await?
    {
        return Ok(None);
    }

    if main_modes::handle_catalog_modes_if_requested(&cli, config)? {
        return Ok(None);
    }
    if main_resume::handle_resume_list_if_requested(&cli, config).await? {
        return Ok(None);
    }

    let mut resume_messages = main_resume::load_explicit_resume_messages(&cli, config)?;
    main_resume::maybe_continue_session(&cli, config, &mut resume_messages);
    main_resume::maybe_auto_resume(&cli, config, &mut resume_messages);
    if main_modes::handle_session_management_if_requested(&cli, config)? {
        return Ok(None);
    }
    if main_modes::handle_background_if_requested(&cli)? {
        return Ok(None);
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
        return Ok(None);
    }
    main_modes::ensure_interactive_tty()?;

    Ok(Some((cli, resume_messages)))
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
