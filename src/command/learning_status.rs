//! v0.1.23: /learning-status slash command handler.
//!
//! Reports status of all 8 learning subsystems: AutoCapture, AutoExtraction,
//! SONA, DESC, GNN, CausalMemory, ShadowVector, ReasoningBank, + Reflexion.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct LearningStatusHandler;

impl CommandHandler for LearningStatusHandler {
    fn description(&self) -> &str {
        "Report status of all learning subsystems (SONA, DESC, ReasoningBank, etc.)"
    }

    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let status = match archon_core::config::load_config() {
            Ok(config) => format!(
                "## Learning Systems Status (v0.1.23)\n\
                 \n\
                 | Subsystem         | Status  |\n\
                 |-------------------|---------|\n\
                 | SONA              | {} |\n\
                 | DESC              | {} |\n\
                 | GNN               | {} |\n\
                 | Causal Memory     | {} |\n\
                 | Shadow Vector     | {} |\n\
                 | Reasoning Bank    | {} |\n\
                 | AutoCapture       | {} |\n\
                 | AutoExtraction    | {} |\n\
                 | Reflexion         | {} |\n\
                 \n\
                 AutoExtraction interval: every {} turns.\n\
                 Reflexion max failures per agent: {}.",
                on_off(config.learning.sona.enabled),
                on_off(config.learning.desc.enabled),
                on_off(config.learning.gnn.enabled),
                on_off(config.learning.causal_memory.enabled),
                on_off(config.learning.shadow_vector.enabled),
                on_off(config.learning.reasoning_bank.enabled),
                on_off(config.memory.auto_capture.enabled),
                on_off(config.memory.auto_extraction.enabled),
                on_off(config.learning.reflexion.enabled),
                config.memory.auto_extraction.every_n_turns,
                config.learning.reflexion.max_per_agent,
            ),
            Err(e) => format!(
                "## Learning Systems Status (v0.1.23)\n\nConfig unavailable: {e}\n\n\
                 All learning subsystems are configured via `~/.archon/config.toml`."
            ),
        };

        let _ = ctx.tui_tx.send(TuiEvent::TextDelta(status));
        Ok(())
    }
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    #[test]
    fn learning_status_smoke_emits_text_delta() {
        let (mut ctx, mut rx) = CtxBuilder::new().build();
        LearningStatusHandler
            .execute(&mut ctx, &[])
            .expect("execute must succeed");
        let events = drain_tui_events(&mut rx);
        let has_table = events.iter().any(|e| match e {
            TuiEvent::TextDelta(s) => s.contains("SONA") && s.contains("Learning Systems Status"),
            _ => false,
        });
        assert!(has_table, "must emit learning status table");
    }

    #[test]
    fn learning_status_handler_has_description() {
        let desc = LearningStatusHandler.description();
        assert!(desc.contains("learning"), "description must mention learning, got: {desc}");
    }
}
