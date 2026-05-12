use archon_core::agent::AgentEvent;
use archon_core::env_vars::ArchonEnvVars;
use archon_core::print_mode::{PrintModeConfig, run_print_mode};
use archon_core::remote::protocol::AgentMessage;

use super::{BuiltAgent, agent_ledger, build_session_agent, open_governed_learning_db};
use crate::cli_args::Cli;

/// Run a print-mode session: set up auth/agent, process one query, return exit code.
pub(crate) async fn run_print_mode_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    print_config: PrintModeConfig,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
) -> i32 {
    let BuiltAgent {
        mut agent,
        event_rx,
        agent_def,
        selected_provider,
        selected_model,
        permission_mode,
        ..
    } = match build_session_agent(config, session_id, cli, env_vars, resolved_flags, true).await {
        Ok(b) => b,
        Err(exit_code) => return exit_code,
    };

    let mut print_config = print_config;
    if let Some(ref def) = agent_def
        && let Some(ref prefix) = def.initial_prompt
    {
        print_config.query = format!("{prefix}\n\n{}", print_config.query);
    }

    let working_dir = std::env::current_dir().unwrap_or_default();
    let governed_learning_db = open_governed_learning_db(&working_dir);
    let ledger_context = agent_ledger::context(
        session_id,
        agent_def.as_ref(),
        selected_model,
        selected_provider,
    );
    let event_rx = agent_ledger::spawn_print_forwarder(
        event_rx,
        governed_learning_db,
        ledger_context,
        permission_mode,
    );

    run_print_mode(print_config, config, &mut agent, event_rx).await
}

/// Run a headless-mode session over JSON-lines stdin/stdout.
#[allow(dead_code)]
pub(crate) async fn run_headless_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    env_vars: &ArchonEnvVars,
    resolved_flags: &archon_core::cli_flags::ResolvedFlags,
) -> i32 {
    let BuiltAgent {
        mut agent,
        event_rx,
        agent_def,
        selected_provider,
        selected_model,
        permission_mode,
        ..
    } = match build_session_agent(config, session_id, cli, env_vars, resolved_flags, false).await {
        Ok(b) => b,
        Err(exit_code) => return exit_code,
    };

    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    let mut event_rx = event_rx;
    let working_dir = std::env::current_dir().unwrap_or_default();
    let governed_learning_db = open_governed_learning_db(&working_dir);
    let ledger_context = agent_ledger::context(
        session_id,
        agent_def.as_ref(),
        selected_model,
        selected_provider,
    );

    tracing::info!(%session_id, "headless: agent loop started");

    loop {
        line.clear();
        match tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await {
            Ok(0) => {
                tracing::info!("headless: stdin closed (EOF)");
                break;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("headless: read error: {e}");
                return 1;
            }
        }

        match AgentMessage::from_json_line(&line) {
            Ok(AgentMessage::Ping) => {
                if write_line(&mut stdout, AgentMessage::Pong).await.is_err() {
                    tracing::error!("headless: stdout write failed, exiting");
                    return 1;
                }
            }
            Ok(AgentMessage::UserMessage { content }) => {
                if !process_headless_message(
                    &mut agent,
                    &mut event_rx,
                    &mut stdout,
                    &content,
                    governed_learning_db.as_ref(),
                    &ledger_context,
                    &permission_mode,
                )
                .await
                {
                    return 1;
                }
            }
            Ok(_) => tracing::debug!("headless: ignoring non-UserMessage/non-Ping"),
            Err(e) => {
                tracing::warn!(%e, "headless: parse error");
                if write_line(
                    &mut stdout,
                    AgentMessage::Error {
                        message: format!("parse error: {e}"),
                    },
                )
                .await
                .is_err()
                {
                    return 1;
                }
            }
        }
    }

    0
}

async fn process_headless_message(
    agent: &mut archon_core::agent::Agent,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<archon_core::agent::TimestampedEvent>,
    stdout: &mut tokio::io::Stdout,
    content: &str,
    governed_learning_db: Option<&std::sync::Arc<cozo::DbInstance>>,
    ledger_context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    permission_mode: &std::sync::Arc<tokio::sync::Mutex<String>>,
) -> bool {
    tracing::info!(len = content.len(), "headless: processing UserMessage");

    if let Err(e) = agent.process_message(content).await {
        tracing::error!(%e, "headless: agent error");
        crate::runtime::agent_ledger_events::record_agent_runtime_error(
            governed_learning_db,
            ledger_context,
            &permission_mode.lock().await.clone(),
        );
        drain_stale_events(event_rx);
        return write_line(
            stdout,
            AgentMessage::Error {
                message: format!("agent error: {e}"),
            },
        )
        .await
        .is_ok();
    }

    let mut response_text = String::new();
    loop {
        match event_rx.try_recv() {
            Ok(ts) => {
                agent_ledger::record_event(
                    governed_learning_db,
                    ledger_context,
                    permission_mode,
                    &ts.inner,
                )
                .await;
                if let AgentEvent::TextDelta(text) = ts.inner {
                    response_text.push_str(&text);
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                tracing::warn!("headless: event channel disconnected");
                break;
            }
        }
    }

    write_line(
        stdout,
        AgentMessage::AssistantMessage {
            content: response_text,
        },
    )
    .await
    .is_ok()
}

fn drain_stale_events(
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<archon_core::agent::TimestampedEvent>,
) {
    loop {
        match event_rx.try_recv() {
            Ok(_) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
}

async fn write_line(stdout: &mut tokio::io::Stdout, msg: AgentMessage) -> std::io::Result<()> {
    let line = msg
        .to_json_line()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    tokio::io::AsyncWriteExt::write_all(stdout, line.as_bytes()).await?;
    tokio::io::AsyncWriteExt::flush(stdout).await
}
