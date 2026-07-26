use archon_core::agent::AgentEvent;
use archon_core::env_vars::ArchonEnvVars;
use archon_core::print_mode::{PrintModeConfig, run_print_mode};
use archon_core::remote::protocol::AgentMessage;

use super::build_agent::build_session_agent;
use super::{BuiltAgent, agent_ledger, open_governed_learning_db};
use crate::cli_args::Cli;

trait NonInteractiveGuardTarget {
    fn activate_guardrail(&mut self, session_id: &str, action_id: &str);
    fn set_guardrail_action_id(&mut self, action_id: Option<String>);
    fn set_turn_requirement_reminder(&mut self, reminder: Option<String>);
}

impl NonInteractiveGuardTarget for archon_core::agent::Agent {
    fn activate_guardrail(&mut self, session_id: &str, action_id: &str) {
        crate::command::world_model::activate_guardrail_for_action(session_id, action_id);
    }

    fn set_guardrail_action_id(&mut self, action_id: Option<String>) {
        archon_core::agent::Agent::set_guardrail_action_id(self, action_id);
    }

    fn set_turn_requirement_reminder(&mut self, reminder: Option<String>) {
        archon_core::agent::Agent::set_turn_requirement_reminder(self, reminder);
    }
}

fn apply_non_interactive_guard_scope(
    target: &mut impl NonInteractiveGuardTarget,
    session_id: &str,
    action_id: &str,
    reminder: Option<String>,
) {
    target.activate_guardrail(session_id, action_id);
    target.set_guardrail_action_id(Some(action_id.to_string()));
    target.set_turn_requirement_reminder(reminder);
}

fn clear_non_interactive_guard_scope(target: &mut impl NonInteractiveGuardTarget) {
    target.set_guardrail_action_id(None);
    target.set_turn_requirement_reminder(None);
}

fn begin_non_interactive_guardrail(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    content: &str,
    action_prefix: &str,
) -> Option<crate::command::world_model::RuntimeGuardrailRecord> {
    let task_class = archon_world_model::guardrail::classify_task(
        content,
        archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
    );
    let surface = match task_class {
        archon_world_model::RuntimeTaskClass::CodingChange
        | archon_world_model::RuntimeTaskClass::Debugging
        | archon_world_model::RuntimeTaskClass::Refactor => {
            archon_world_model::integration::WorldAdvisorSurface::CodingTask
        }
        _ => archon_world_model::integration::WorldAdvisorSurface::InteractiveSession,
    };
    crate::command::world_model::begin_guarded_action(
        config,
        surface,
        session_id,
        &format!("{action_prefix}-{}", uuid::Uuid::new_v4()),
        content,
    )
}

fn apply_guardrail_record(
    target: &mut impl NonInteractiveGuardTarget,
    session_id: &str,
    record: &crate::command::world_model::RuntimeGuardrailRecord,
) {
    let action_id = &record.action.action_id;
    let reminder = crate::command::world_model::turn_requirements_for_action(session_id, action_id);
    apply_non_interactive_guard_scope(target, session_id, action_id, reminder);
}

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
        sandbox_audit_drain,
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
    let governed_learning_db = open_governed_learning_db(&working_dir).await;
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

    let guardrail =
        begin_non_interactive_guardrail(config, session_id, &print_config.query, "print-turn");
    if let Some(record) = &guardrail {
        apply_guardrail_record(&mut agent, session_id, record);
    }
    let exit_code = run_print_mode(print_config, config, &mut agent, event_rx).await;
    clear_non_interactive_guard_scope(&mut agent);
    if let Some(record) = &guardrail {
        crate::command::world_model::record_guardrail_turn_outcome(config, record, exit_code == 0);
    }
    if let Err(error) = drain_sandbox_audit(sandbox_audit_drain).await {
        tracing::error!(%error, "sandbox audit writer shutdown failed");
        return 1;
    }
    exit_code
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
        sandbox_audit_drain,
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
    let governed_learning_db = open_governed_learning_db(&working_dir).await;
    let ledger_context = agent_ledger::context(
        session_id,
        agent_def.as_ref(),
        selected_model,
        selected_provider,
    );

    tracing::info!(%session_id, "headless: agent loop started");

    let exit_code = 'headless: loop {
        line.clear();
        match tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await {
            Ok(0) => {
                tracing::info!("headless: stdin closed (EOF)");
                break 'headless 0;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("headless: read error: {e}");
                break 'headless 1;
            }
        }

        match AgentMessage::from_json_line(&line) {
            Ok(AgentMessage::Ping) => {
                if write_line(&mut stdout, AgentMessage::Pong).await.is_err() {
                    tracing::error!("headless: stdout write failed, exiting");
                    break 'headless 1;
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
                    config,
                    session_id,
                )
                .await
                {
                    break 'headless 1;
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
                    break 'headless 1;
                }
            }
        }
    };

    if let Err(error) = drain_sandbox_audit(sandbox_audit_drain).await {
        tracing::error!(%error, "sandbox audit writer shutdown failed");
        return 1;
    }
    exit_code
}

async fn drain_sandbox_audit(
    drain: crate::runtime::sandbox_audit_writer::SandboxAuditDrain,
) -> anyhow::Result<()> {
    let readback = drain.shutdown(std::time::Duration::from_secs(30)).await?;
    tracing::info!(
        accepted = readback.accepted,
        dropped = readback.dropped,
        persisted = readback.persisted,
        failed = readback.failed,
        "sandbox audit writer drained"
    );
    Ok(())
}

async fn record_headless_event(
    timestamped: archon_core::agent::TimestampedEvent,
    response_text: &mut String,
    governed_learning_db: Option<&std::sync::Arc<cozo::DbInstance>>,
    ledger_context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    permission_mode: &std::sync::Arc<tokio::sync::Mutex<String>>,
) {
    agent_ledger::record_event(
        governed_learning_db,
        ledger_context,
        permission_mode,
        &timestamped.inner,
    )
    .await;
    if let AgentEvent::TextDelta(text) = timestamped.inner {
        response_text.push_str(&text);
    }
}

async fn drain_headless_events_while<F, T>(
    future: F,
    event_rx: &mut tokio::sync::mpsc::Receiver<archon_core::agent::TimestampedEvent>,
    response_text: &mut String,
    governed_learning_db: Option<&std::sync::Arc<cozo::DbInstance>>,
    ledger_context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    permission_mode: &std::sync::Arc<tokio::sync::Mutex<String>>,
) -> T
where
    F: std::future::Future<Output = T>,
{
    tokio::pin!(future);
    let output = loop {
        tokio::select! {
            output = &mut future => break output,
            event = event_rx.recv() => {
                let Some(event) = event else {
                    break (&mut future).await;
                };
                record_headless_event(
                    event,
                    response_text,
                    governed_learning_db,
                    ledger_context,
                    permission_mode,
                )
                .await;
            }
        }
    };
    while let Ok(event) = event_rx.try_recv() {
        record_headless_event(
            event,
            response_text,
            governed_learning_db,
            ledger_context,
            permission_mode,
        )
        .await;
    }
    output
}

async fn process_headless_message(
    agent: &mut archon_core::agent::Agent,
    event_rx: &mut tokio::sync::mpsc::Receiver<archon_core::agent::TimestampedEvent>,
    stdout: &mut tokio::io::Stdout,
    content: &str,
    governed_learning_db: Option<&std::sync::Arc<cozo::DbInstance>>,
    ledger_context: &crate::runtime::agent_ledger_events::AgentLedgerContext,
    permission_mode: &std::sync::Arc<tokio::sync::Mutex<String>>,
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
) -> bool {
    tracing::info!(len = content.len(), "headless: processing UserMessage");

    let guardrail = begin_non_interactive_guardrail(config, session_id, content, "headless-turn");
    if let Some(record) = &guardrail {
        apply_guardrail_record(agent, session_id, record);
    }
    let mut response_text = String::new();
    let process_result = drain_headless_events_while(
        agent.process_message(content),
        event_rx,
        &mut response_text,
        governed_learning_db,
        ledger_context,
        permission_mode,
    )
    .await;
    clear_non_interactive_guard_scope(agent);
    if let Some(record) = &guardrail {
        crate::command::world_model::record_guardrail_turn_outcome(
            config,
            record,
            process_result.is_ok(),
        );
    }

    if let Err(e) = process_result {
        tracing::error!(%e, "headless: agent error");
        crate::runtime::agent_ledger_events::record_agent_runtime_error(
            governed_learning_db,
            ledger_context,
            &permission_mode.lock().await.clone(),
        );
        return write_line(
            stdout,
            AgentMessage::Error {
                message: format!("agent error: {e}"),
            },
        )
        .await
        .is_ok();
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

async fn write_line(stdout: &mut tokio::io::Stdout, msg: AgentMessage) -> std::io::Result<()> {
    let line = msg
        .to_json_line()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    tokio::io::AsyncWriteExt::write_all(stdout, line.as_bytes()).await?;
    tokio::io::AsyncWriteExt::flush(stdout).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bounded_headless_source_is_drained_while_agent_future_runs() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let producer = async move {
            for text in ["one", "two", "three"] {
                tx.send(archon_core::agent::TimestampedEvent {
                    sent_at: std::time::Instant::now(),
                    inner: AgentEvent::TextDelta(text.into()),
                })
                .await
                .expect("bounded event send");
            }
            Ok::<_, anyhow::Error>(())
        };

        let ledger_context = crate::runtime::agent_ledger_events::AgentLedgerContext::new(
            "main",
            "headless-test",
            "test-model",
            "test-provider",
        );
        let permission_mode = std::sync::Arc::new(tokio::sync::Mutex::new("auto".into()));
        let mut response_text = String::new();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            drain_headless_events_while(
                producer,
                &mut rx,
                &mut response_text,
                None,
                &ledger_context,
                &permission_mode,
            ),
        )
        .await
        .expect("bounded source must not deadlock");

        result.expect("producer result");
        assert_eq!(response_text, "onetwothree");
    }

    #[derive(Default)]
    struct RecordingGuardTarget {
        activated: Option<(String, String)>,
        action_id: Option<String>,
        reminder: Option<String>,
    }

    impl NonInteractiveGuardTarget for RecordingGuardTarget {
        fn activate_guardrail(&mut self, session_id: &str, action_id: &str) {
            self.activated = Some((session_id.to_string(), action_id.to_string()));
        }

        fn set_guardrail_action_id(&mut self, action_id: Option<String>) {
            self.action_id = action_id;
        }

        fn set_turn_requirement_reminder(&mut self, reminder: Option<String>) {
            self.reminder = reminder;
        }
    }

    #[test]
    fn non_interactive_guard_scope_activates_and_assigns_action_identity() {
        let mut target = RecordingGuardTarget::default();

        apply_non_interactive_guard_scope(
            &mut target,
            "print-session",
            "print-turn-1",
            Some("run required verification".into()),
        );

        assert_eq!(
            target.activated,
            Some(("print-session".into(), "print-turn-1".into()))
        );
        assert_eq!(target.action_id.as_deref(), Some("print-turn-1"));
        assert_eq!(
            target.reminder.as_deref(),
            Some("run required verification")
        );

        clear_non_interactive_guard_scope(&mut target);

        assert!(target.action_id.is_none());
        assert!(target.reminder.is_none());
    }
}
