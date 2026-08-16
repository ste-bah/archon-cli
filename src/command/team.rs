use std::sync::Arc;

use archon_core::agents::AgentRegistry;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_core::orchestrator::{Orchestrator, RealSubtaskExecutor};

use crate::cli_args::TeamAction;
use crate::runtime::llm::build_configured_llm_provider;

/// The roles a team's roster declares, in declaration order.
///
/// An empty roster is refused rather than run: `run_team` would plan zero
/// subtasks, execute nothing, and print a successful-looking result.
fn load_team_agents(team_id: &str) -> Result<Vec<String>, String> {
    use archon_core::team::TeamManager;

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let manager = TeamManager::new(cwd);
    let config = manager.load_team(team_id).map_err(|e| {
        format!(
            "Team '{team_id}' not found in {}: {e}\nRun `archon team list` to see what is there.",
            manager.teams_dir().display()
        )
    })?;

    let agents: Vec<String> = config.members.into_iter().map(|m| m.role).collect();
    if agents.is_empty() {
        return Err(format!("Team '{team_id}' has no members — nothing to run."));
    }
    Ok(agents)
}

pub(crate) async fn handle_team_command(
    action: &TeamAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> anyhow::Result<()> {
    match action {
        TeamAction::Run { team, goal } => {
            let orch = Orchestrator::new(config.orchestrator.clone());
            let (tx, mut rx) = tokio::sync::mpsc::channel(64);
            let team_provider = build_configured_llm_provider(config, env_vars, "team")
                .await
                .map_err(|e| anyhow::anyhow!("Authentication failed for team execution: {e}"))?;
            let cwd = std::env::current_dir().unwrap_or_default();
            let team_agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(&cwd)));
            let session_store = Arc::new(crate::command::store_paths::open_session_store(
                &crate::command::store_paths::session_db_path(config),
            )?);
            let executor = Arc::new(RealSubtaskExecutor::new(
                team_provider,
                cwd,
                config.api.default_model.clone(),
                team_agent_registry,
                session_store,
            ));
            // The roster is what says who is on the team. Building the config
            // with `..Default::default()` left `agents` empty, and `run_team`
            // plans one subtask per agent — so every run decomposed to zero
            // subtasks and reported success having done nothing (#184 M5).
            let team_cfg = match load_team_agents(team) {
                Ok(agents) => archon_core::orchestrator::config::TeamConfig {
                    name: team.clone(),
                    agents,
                    ..Default::default()
                },
                Err(problem) => {
                    eprintln!("{problem}");
                    std::process::exit(1);
                }
            };
            // Milestone 2 topology tap. `OrchestratorEvent` has no subscriber
            // registry — `Orchestrator::run_team` takes one `mpsc::Sender` and
            // nothing fans it out — so this receiver loop is the seam. Tracing
            // here costs one file append per event and no database access.
            let trace_root = std::env::current_dir().unwrap_or_default();
            let trace_graph_id = format!("team-{}", team_cfg.name);
            crate::command::topology_trace::begin(&trace_root, &trace_graph_id, &trace_graph_id);
            // Milestone 3: track the same id for guardrail admission, so the
            // team's declared graph, node lifecycle, and write claims are all
            // keyed together. Enforcement is governed by `[topology]`; with
            // `admission_enabled = false` this is a no-op.
            crate::command::topology_admission::install(config, &trace_graph_id);

            archon_observability::spawn_named("team-event-printer", async move {
                while let Some(event) = rx.recv().await {
                    use archon_core::orchestrator::events::OrchestratorEvent;
                    crate::command::topology_trace::on_orchestrator_event(&event);
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
            let run = orch.run_team(team_cfg, goal.clone(), executor, tx).await;

            // Graph completion: fold the ambient trace. `spawn_blocking`
            // because the fold is synchronous and the Cozo guard's retry loop
            // sleeps on `thread::sleep` — up to ~19 seconds — which on a tokio
            // worker is a runtime stall. One writer, batched, one transaction.
            let fold_goal = goal.clone();
            let fold_graph_id = trace_graph_id.clone();
            let fold_graph_id_for_end = trace_graph_id.clone();
            let _ = tokio::task::spawn_blocking(move || {
                crate::command::topology_fold::fold_project_pending_blocking(
                    &trace_root,
                    &fold_graph_id,
                    &fold_goal,
                    "default",
                )
            })
            .await;
            crate::command::topology_trace::end();
            // Bounded state, dropped at session end.
            crate::command::topology_admission::end_session(&fold_graph_id_for_end);

            match run {
                Ok(result) => println!("Result: {result}"),
                Err(e) => {
                    eprintln!("Team run failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        TeamAction::List => {
            use archon_core::team::TeamManager;
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let manager = TeamManager::new(cwd.clone());
            match manager.list_teams() {
                Ok(ids) if ids.is_empty() => {
                    println!("No teams found in {}", manager.teams_dir().display());
                }
                Ok(ids) => {
                    println!("Teams ({}):", ids.len());
                    for id in ids {
                        match manager.load_team(&id) {
                            Ok(cfg) => println!(
                                "  {id:<24}  {name}  ({n} member{s}, {filled} running)",
                                name = cfg.name,
                                n = cfg.members.len(),
                                s = if cfg.members.len() == 1 { "" } else { "s" },
                                filled = cfg.members.iter().filter(|m| m.is_filled()).count(),
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
