use std::path::PathBuf;

use anyhow::Result;
use archon_core::cli_flags::ResolvedFlags;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;

use crate::cli_args::{AuthArgs, AuthProviderKind, AuthSubcommand, Cli, Commands};

pub(crate) async fn handle_subcommand(
    command: Commands,
    cli: &Cli,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    resolved_flags: &ResolvedFlags,
    working_dir_for_config: &PathBuf,
) -> Result<()> {
    match command {
        command @ (Commands::Login
        | Commands::Auth(_)
        | Commands::Chat(_)
        | Commands::Providers { .. }
        | Commands::Sandbox { .. }
        | Commands::Permissions { .. }
        | Commands::Plugin { .. }
        | Commands::Update { .. }
        | Commands::Remote { .. }
        | Commands::Serve { .. }
        | Commands::Team { .. }
        | Commands::IdeStdio
        | Commands::Web { .. }) => {
            handle_runtime_command(command, cli, config, env_vars, resolved_flags).await
        }
        command @ (Commands::Behaviour { .. }
        | Commands::Agent { .. }
        | Commands::Learning { .. }
        | Commands::World { .. }
        | Commands::Reasoning { .. }
        | Commands::Cognitive { .. }
        | Commands::Briefing { .. }
        | Commands::Pipeline { .. }
        | Commands::Workflow { .. }) => handle_learning_command(command, config, env_vars).await,
        command @ (Commands::RunAgentAsync { .. }
        | Commands::TaskStatus { .. }
        | Commands::TaskResult { .. }
        | Commands::TaskCancel { .. }
        | Commands::TaskList { .. }
        | Commands::TaskEvents { .. }
        | Commands::Metrics
        | Commands::AgentList { .. }
        | Commands::AgentSearch { .. }
        | Commands::AgentInfo { .. }) => handle_task_command(command, working_dir_for_config).await,
        command @ (Commands::Kb { .. }
        | Commands::Docs { .. }
        | Commands::Video { .. }
        | Commands::Trading { .. }
        | Commands::Prov { .. }
        | Commands::Meaning { .. }
        | Commands::Constellation { .. }
        | Commands::Memory { .. }) => handle_data_command(command).await,
        command @ (Commands::SelfCmd { .. }
        | Commands::Gametheory { .. }
        | Commands::Completion { .. }) => handle_analysis_command(command, config, env_vars).await,
    }
}

async fn handle_runtime_command(
    command: Commands,
    cli: &Cli,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    resolved_flags: &ResolvedFlags,
) -> Result<()> {
    match command {
        Commands::Login => {
            crate::command::auth::handle_auth(
                AuthArgs {
                    command: AuthSubcommand::Login {
                        provider: AuthProviderKind::Anthropic,
                        accept_tos: true,
                    },
                },
                config,
            )
            .await
        }
        Commands::Auth(args) => crate::command::auth::handle_auth(args, config).await,
        Commands::Chat(args) => crate::command::chat::handle_chat(args, config).await,
        Commands::Providers { action } => {
            crate::command::providers::handle_providers(action, config)
        }
        Commands::Sandbox { action } => {
            crate::command::sandbox_cli::handle_sandbox_command(action, config)
        }
        Commands::Permissions { action } => {
            crate::command::permissions_cli::handle_permissions_command(&action)
        }
        Commands::Plugin { action } => crate::command::plugin::handle_plugin_command(action),
        Commands::Update { check, force } => {
            crate::command::update::handle_update_command(check, force, config).await
        }
        Commands::Remote { .. } | Commands::Serve { .. } => {
            crate::command::remote::handle_remote_command(cli, config).await
        }
        Commands::Team { action } => {
            crate::command::team::handle_team_command(&action, config, env_vars).await
        }
        Commands::IdeStdio => crate::command::ide_stdio::handle_ide_stdio_command().await,
        Commands::Web {
            port,
            bind_address,
            no_open,
            allow_unauthenticated_nonlocal_bind,
        } => {
            crate::command::web::handle_web_command(
                port,
                bind_address,
                no_open,
                allow_unauthenticated_nonlocal_bind,
                config,
                cli,
                env_vars,
                resolved_flags,
            )
            .await
        }
        _ => unreachable!("runtime command routed to wrong dispatcher"),
    }
}

async fn handle_learning_command(
    command: Commands,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    match command {
        Commands::Behaviour { action } => {
            crate::command::behaviour::handle_behaviour_command(&action, config).await
        }
        Commands::Agent { action } => {
            crate::command::agent_evolve::handle_agent_command(&action, config).await
        }
        Commands::Learning { action } => {
            crate::command::learning::handle_learning_command(action, config).await
        }
        Commands::World { action } => {
            crate::command::world_model::handle_world_command(&action, config, env_vars).await
        }
        Commands::Reasoning { action } => {
            crate::command::reasoning::handle_reasoning_command(&action, config).await
        }
        Commands::Cognitive { action } => {
            crate::command::cognitive::handle_cognitive_command(&action, config).await
        }
        Commands::Briefing { action } => {
            crate::command::reasoning::handle_briefing_command(&action, config).await
        }
        Commands::Pipeline { action } => {
            crate::command::pipeline::handle_pipeline_command(&action, config, env_vars).await
        }
        Commands::Workflow { action } => {
            crate::command::workflow::handle_workflow_command(&action, config, env_vars).await
        }
        _ => unreachable!("learning command routed to wrong dispatcher"),
    }
}

async fn handle_task_command(command: Commands, working_dir_for_config: &PathBuf) -> Result<()> {
    match command {
        Commands::RunAgentAsync {
            name,
            input,
            version,
            detach,
        } => {
            crate::command::task::handle_run_agent_async(
                name,
                input,
                version,
                detach,
                working_dir_for_config,
            )
            .await
        }
        Commands::TaskStatus { task_id, watch } => {
            crate::command::task::handle_task_status(&task_id, watch, working_dir_for_config).await
        }
        Commands::TaskResult { task_id, stream } => {
            crate::command::task::handle_task_result(&task_id, stream, working_dir_for_config).await
        }
        Commands::TaskCancel { task_id } => {
            crate::command::task::handle_task_cancel(&task_id, working_dir_for_config).await
        }
        Commands::TaskList {
            state,
            agent,
            since,
        } => {
            crate::command::task::handle_task_list(state, agent, since, working_dir_for_config)
                .await
        }
        Commands::TaskEvents { task_id, from_seq } => {
            crate::command::task::handle_task_events(&task_id, from_seq, working_dir_for_config)
                .await
        }
        Commands::Metrics => crate::command::task::handle_metrics(working_dir_for_config).await,
        Commands::AgentList { include_invalid } => {
            crate::command::agent::handle_agent_list(include_invalid, working_dir_for_config).await
        }
        Commands::AgentSearch {
            tags,
            capabilities,
            name_pattern,
            version,
            logic,
            include_invalid,
            registry_url,
        } => {
            crate::command::agent::handle_agent_search(
                tags,
                capabilities,
                name_pattern,
                version,
                logic,
                include_invalid,
                registry_url,
                working_dir_for_config,
            )
            .await
        }
        Commands::AgentInfo {
            name,
            version,
            json,
        } => {
            crate::command::agent::handle_agent_info(name, version, json, working_dir_for_config)
                .await
        }
        _ => unreachable!("task command routed to wrong dispatcher"),
    }
}

async fn handle_data_command(command: Commands) -> Result<()> {
    match command {
        Commands::Kb { action } => crate::command::kb::handle_kb_command(action).await,
        Commands::Docs { action } => crate::command::docs::handle_docs_command(action).await,
        Commands::Video { action } => crate::command::video::handle_video_command(action).await,
        Commands::Trading { action } => crate::command::trading::handle_trading_command(&action),
        Commands::Prov { action } => crate::command::prov::handle_prov_command(action).await,
        Commands::Meaning { action } => {
            crate::command::meaning::handle_meaning_command(action).await
        }
        Commands::Constellation { action } => {
            crate::command::constellation::handle_constellation_command(action).await
        }
        Commands::Memory { action } => {
            crate::command::memory_cli::handle_memory_command(action).await
        }
        _ => unreachable!("data command routed to wrong dispatcher"),
    }
}

async fn handle_analysis_command(
    command: Commands,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    match command {
        Commands::SelfCmd { action } => {
            crate::command::self_calibration::handle_self_command(action, config, env_vars).await
        }
        Commands::Gametheory {
            situation,
            classify_only,
            kb,
            spec_path,
            debug_memory,
            budget,
            max_concurrent,
            style,
            enable_tier11,
            action,
        } => {
            crate::command::gametheory::handle_gametheory(
                action.as_ref(),
                situation.as_deref(),
                classify_only,
                kb.as_deref(),
                spec_path.as_deref(),
                debug_memory,
                budget,
                max_concurrent,
                &style,
                enable_tier11,
                config,
                env_vars,
            )
            .await
        }
        Commands::Completion { action } => {
            crate::command::completion::handle_completion(&action).await
        }
        _ => unreachable!("analysis command routed to wrong dispatcher"),
    }
}
