use std::sync::Arc;

use archon_core::agents::AgentRegistry;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_core::orchestrator::{Orchestrator, RealSubtaskExecutor};

use crate::cli_args::TeamAction;
use crate::runtime::llm::build_llm_provider;

pub(crate) async fn handle_team_command(
    action: &TeamAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> anyhow::Result<()> {
    match action {
        TeamAction::Run { team, goal } => {
            let orch = Orchestrator::new(config.orchestrator.clone());
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            // Build LLM provider for team execution
            let team_auth = match archon_llm::auth::resolve_auth_with_keys(
                env_vars.anthropic_api_key.as_deref(),
                env_vars.archon_api_key.as_deref(),
                env_vars.archon_oauth_token.as_deref(),
                std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
            ) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("Authentication failed for team execution: {e}");
                    eprintln!("Run `archon login` or set ANTHROPIC_API_KEY.");
                    std::process::exit(1);
                }
            };
            let team_identity = archon_llm::identity::IdentityProvider::new(
                archon_llm::identity::IdentityMode::Clean,
                uuid::Uuid::new_v4().to_string(),
                "team-device".to_string(),
                String::new(),
            );
            let team_api_url =
                std::env::var("ANTHROPIC_BASE_URL").ok().or_else(|| config.api.base_url.clone());
            let team_client = archon_llm::anthropic::AnthropicClient::new(
                team_auth,
                team_identity,
                team_api_url,
            );
            let team_provider = build_llm_provider(&config.llm, team_client);
            let cwd = std::env::current_dir().unwrap_or_default();
            let team_agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(&cwd)));
            let executor = Arc::new(RealSubtaskExecutor::new(
                team_provider,
                cwd,
                config.api.default_model.clone(),
                team_agent_registry,
            ));
            let team_cfg = archon_core::orchestrator::config::TeamConfig {
                name: team.clone(),
                ..Default::default()
            };
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    use archon_core::orchestrator::events::OrchestratorEvent;
                    match event {
                        OrchestratorEvent::TaskDecomposed { subtasks } => {
                            println!("  Plan: {} subtasks", subtasks.len());
                        }
                        OrchestratorEvent::AgentSpawned {
                            agent_type,
                            subtask_id,
                            ..
                        } => {
                            println!("  [spawn] {agent_type} → subtask {subtask_id}");
                        }
                        OrchestratorEvent::AgentComplete { subtask_id, .. } => {
                            println!("  [done] subtask {subtask_id}");
                        }
                        OrchestratorEvent::TeamComplete { result } => {
                            println!("Team complete:\n{result}");
                        }
                        _ => {}
                    }
                }
            });
            match orch.run_team(team_cfg, goal.clone(), executor, tx).await {
                Ok(result) => println!("Result: {result}"),
                Err(e) => {
                    eprintln!("Team run failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        TeamAction::List => {
            use archon_core::team::TeamManager;
            let cwd =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let manager = TeamManager::new(cwd.clone());
            match manager.list_teams() {
                Ok(ids) if ids.is_empty() => {
                    println!("No teams found in {}/teams", cwd.display());
                }
                Ok(ids) => {
                    println!("Teams ({}):", ids.len());
                    for id in ids {
                        match manager.load_team(&id) {
                            Ok(cfg) => println!(
                                "  {id:<24}  {name}  ({n} members)",
                                name = cfg.name,
                                n = cfg.members.len()
                            ),
                            Err(e) => {
                                println!("  {id:<24}  <unreadable team.json: {e}>")
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to list teams: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}
