//! TASK-HOTFIX-V0.1.7: /run-agent slash-command handler (#248).
//!
//! Registered as a primary command so `/run-agent <name> <task>` no longer
//! produces "Unknown command". Validates the agent name against the registry
//! and emits actionable instructions.
//!
//! Full async agent invocation (Option A) requires plumbing an async task
//! submission surface into the sync `CommandHandler::execute` path — deferred
//! to a follow-up feature ticket. This handler fixes the immediate UX break:
//! `/agent run` hints no longer point to a non-existent command.

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/run-agent` command.
pub(crate) struct RunAgentHandler;

impl CommandHandler for RunAgentHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        if args.is_empty() {
            ctx.emit(archon_tui::app::TuiEvent::TextDelta(
                "\n/run-agent — invoke a custom agent.\n\n\
                 Usage: /run-agent <agent-name> <task description>\n\n\
                 The task is sent as a natural-language instruction. The system\n\
                 routes it to the agent you name.\n\n\
                 Run /agent list to see available agent names.\n"
                    .to_string(),
            ));
            return Ok(());
        }

        let agent_name = &args[0];
        let task = if args.len() > 1 {
            args[1..].join(" ")
        } else {
            String::new()
        };

        // Validate agent exists if registry is available.
        if let Some(ref registry) = ctx.agent_registry {
            let reg = registry.read().unwrap();
            if reg.resolve(agent_name).is_none() {
                ctx.emit(archon_tui::app::TuiEvent::Error(format!(
                    "Unknown agent: {agent_name}. Use /agent list to see registered agents."
                )));
                return Ok(());
            }
        }

        if task.is_empty() {
            ctx.emit(archon_tui::app::TuiEvent::TextDelta(format!(
                "\nAgent `{agent_name}` found. Provide a task description:\n\
                 /run-agent {agent_name} <task description>\n"
            )));
        } else {
            ctx.emit(archon_tui::app::TuiEvent::TextDelta(format!(
                "\nTo invoke `{agent_name}` with this task, type it as a natural-\n\
                 language instruction in the input box:\n\n\
                 > use the {agent_name} agent to {task}\n\n\
                 The system will route your request to the agent automatically.\n"
            )));
        }

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
    fn run_agent_handler_returns_ok() {
        let (mut ctx, _rx) = make_ctx();
        let result = RunAgentHandler.execute(&mut ctx, &[]);
        assert!(result.is_ok(), "execute must return Ok");
    }
}
