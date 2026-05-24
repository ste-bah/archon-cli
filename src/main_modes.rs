use anyhow::Result;
use archon_core::input_format::InputFormat;
use archon_core::output_format::OutputFormat;
use archon_core::print_mode::PrintModeConfig;

use crate::cli_args::{Cli, Commands};

pub(crate) async fn handle_subcommand_if_present(
    cli: &mut Cli,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    working_dir_for_config: &std::path::PathBuf,
) -> Result<bool> {
    if matches!(
        &cli.command,
        Some(Commands::Remote { .. }) | Some(Commands::Serve { .. })
    ) {
        crate::command::remote::handle_remote_command(cli, config).await?;
        return Ok(true);
    }
    let Some(command) = cli.command.take() else {
        return Ok(false);
    };
    crate::main_dispatch::handle_subcommand(
        command,
        cli,
        config,
        env_vars,
        resolved_flags,
        working_dir_for_config,
    )
    .await?;
    Ok(true)
}

pub(crate) async fn handle_headless_if_requested(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    session_id: &str,
) -> Result<bool> {
    if !cli.headless {
        return Ok(false);
    }
    let headless_session_id = cli
        .session_id
        .clone()
        .unwrap_or_else(|| session_id.to_string());
    tracing::info!("headless mode: session_id={headless_session_id}");
    let exit_code = crate::session::run_headless_session(
        config,
        &headless_session_id,
        cli,
        env_vars,
        resolved_flags,
    )
    .await;
    std::process::exit(exit_code);
}

pub(crate) fn handle_catalog_modes_if_requested(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
) -> Result<bool> {
    if cli.list_output_styles {
        crate::command::tui_helpers::handle_list_output_styles()?;
        return Ok(true);
    }
    if cli.list_themes {
        crate::command::tui_helpers::handle_list_themes(cli, config)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn handle_session_management_if_requested(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
) -> Result<bool> {
    if cli.sessions {
        crate::command::sessions::handle_sessions(cli, config)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn handle_background_if_requested(cli: &Cli) -> Result<bool> {
    if cli.ps {
        crate::command::background::handle_bg_list()?;
        return Ok(true);
    }
    if let Some(ref id) = cli.kill_session {
        crate::command::background::handle_bg_kill(id)?;
        return Ok(true);
    }
    if let Some(ref id) = cli.attach {
        crate::command::background::handle_bg_attach(id)?;
        return Ok(true);
    }
    if let Some(ref id) = cli.logs {
        crate::command::background::handle_bg_logs(id)?;
        return Ok(true);
    }
    if cli.bg.is_some() {
        crate::command::background::handle_bg_launch(cli)?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) async fn handle_print_mode_if_requested(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    session_id: &str,
) -> Result<bool> {
    if cli.print.is_none() {
        return Ok(false);
    }
    let print_config = build_print_config(cli)?;
    let exit_code = crate::session::run_print_mode_session(
        config,
        session_id,
        cli,
        env_vars,
        print_config,
        resolved_flags,
    )
    .await;
    std::process::exit(exit_code);
}

pub(crate) fn ensure_interactive_tty() -> Result<()> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        anyhow::bail!(
            "interactive mode requires a TTY; use -p/--print, --headless, or --ide-stdio for non-interactive input"
        );
    }
    Ok(())
}

fn build_print_config(cli: &Cli) -> Result<PrintModeConfig> {
    let query = match &cli.print {
        Some(Some(query)) => query.clone(),
        Some(None) => read_print_stdin(cli)?,
        None => unreachable!(),
    };
    let output_format = OutputFormat::from_str(&cli.output_format).unwrap_or_else(|error| {
        eprintln!("error: {error}");
        std::process::exit(1);
    });
    let json_schema = crate::resolve_json_schema(cli).unwrap_or_else(|error| {
        eprintln!("error: {error}");
        std::process::exit(1);
    });
    Ok(PrintModeConfig {
        query,
        output_format,
        input_format: InputFormat::from_str(&cli.input_format).unwrap_or(InputFormat::Text),
        max_turns: cli.max_turns,
        max_budget_usd: cli.max_budget_usd,
        no_session_persistence: cli.no_session_persistence,
        json_schema,
    })
}

fn read_print_stdin(cli: &Cli) -> Result<String> {
    let input_format = InputFormat::from_str(&cli.input_format).unwrap_or_else(|error| {
        eprintln!("error: {error}");
        std::process::exit(1);
    });
    let messages = archon_core::input_format::read_input(&input_format).unwrap_or_else(|error| {
        eprintln!("error reading input: {error}");
        std::process::exit(1);
    });
    Ok(messages.join("\n"))
}
