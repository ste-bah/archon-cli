//! `/run-agent` slash-command handler.
//!
//! v0.1.8: Wired with real `TaskService::submit()` via `tokio::spawn`.
//! Validates the agent name against the registry, then submits the task
//! asynchronously without blocking the TUI input loop.
//!
//! v0.1.7: Hint-only stub that emitted natural-language instructions.

use archon_core::tasks::models::SubmitRequest;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Handler for `/run-agent <agent-name> <task description>`.
pub(crate) struct RunAgentHandler;

impl CommandHandler for RunAgentHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        if args.is_empty() {
            ctx.emit(TuiEvent::TextDelta(
                "\n/run-agent — invoke a custom agent.\n\n\
                 Usage: /run-agent <agent-name> <task description>\n\n\
                 The task is submitted asynchronously. The agent name must\n\
                 match a registered custom agent.\n\n\
                 Run /agent list to see available agent names.\n"
                    .to_string(),
            ));
            return Ok(());
        }

        let agent_name = args[0].clone();
        let task = if args.len() > 1 {
            args[1..].join(" ")
        } else {
            String::new()
        };

        // Validate agent exists if registry is available.
        if let Some(ref registry) = ctx.agent_registry {
            let reg = registry.read().unwrap();
            if reg.resolve(&agent_name).is_none() {
                ctx.emit(TuiEvent::Error(format!(
                    "Unknown agent: {agent_name}. Use /agent list to see registered agents."
                )));
                return Ok(());
            }
        }

        if task.is_empty() {
            ctx.emit(TuiEvent::TextDelta(format!(
                "\nAgent `{agent_name}` found. Provide a task description:\n\
                 /run-agent {agent_name} <task description>\n"
            )));
            return Ok(());
        }

        // Submit via TaskService asynchronously.
        let task_service = match ctx.task_service.clone() {
            Some(ts) => ts,
            None => {
                ctx.emit(TuiEvent::Error(
                    "TaskService not available (no async runtime configured).".into(),
                ));
                return Ok(());
            }
        };

        let tui_tx = ctx.tui_tx.clone();
        let input = serde_json::Value::String(task.clone());
        let owner = "tui".to_string();

        tokio::spawn(async move {
            match task_service
                .submit(SubmitRequest {
                    agent_name,
                    agent_version: None,
                    input,
                    owner,
                })
                .await
            {
                Ok(task_id) => {
                    let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                        "\nTask submitted: {task_id}\n\
                         Task: {task}\n"
                    )));
                }
                Err(e) => {
                    let _ = tui_tx.send(TuiEvent::Error(format!("Task submission failed: {e}")));
                }
            }
        });

        Ok(())
    }

    fn description(&self) -> &str {
        "Invoke a custom agent by name with a task description"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    fn make_ctx() -> (
        crate::command::registry::CommandContext,
        tokio::sync::mpsc::UnboundedReceiver<TuiEvent>,
    ) {
        CtxBuilder::new().build()
    }

    #[test]
    fn run_agent_handler_no_args_emits_usage() {
        let (mut ctx, mut rx) = make_ctx();
        RunAgentHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let has_text = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Usage:")));
        assert!(has_text, "no-args must emit usage text, got: {:?}", events);
    }

    #[test]
    fn run_agent_handler_description_matches() {
        let desc = RunAgentHandler.description();
        assert!(
            desc.contains("Invoke"),
            "description must mention invoke, got: {desc}"
        );
    }

    #[test]
    fn run_agent_handler_no_task_service_emits_error() {
        let (mut ctx, mut rx) = make_ctx();
        RunAgentHandler
            .execute(&mut ctx, &["sherlock-holmes".into(), "audit".into()])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_error,
            "missing task_service must emit error, got: {:?}",
            events
        );
    }

    #[test]
    fn run_agent_handler_returns_ok() {
        let (mut ctx, _rx) = make_ctx();
        let result = RunAgentHandler.execute(&mut ctx, &[]);
        assert!(result.is_ok(), "execute must return Ok");
    }
}
