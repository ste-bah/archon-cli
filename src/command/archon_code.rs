//! `/archon-code` slash-command handler.
//!
//! Spawns the 48-agent coding pipeline via `tokio::spawn` wrapping
//! `archon_pipeline::runner::run_pipeline()`. Per-agent progress events
//! are streamed to the TUI via the facade's `tui_sender`.

use std::sync::Arc;

use crate::command::registry::{CommandContext, CommandHandler};
use archon_pipeline::coding::facade::CodingFacade;
use archon_pipeline::runner::{LlmClient, run_pipeline};
use archon_tui::app::TuiEvent;

/// Handler for `/archon-code <task description>`.
pub(crate) struct ArchonCodeHandler;

impl CommandHandler for ArchonCodeHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        if args.is_empty() {
            ctx.emit(TuiEvent::TextDelta(
                "\n/archon-code — run the 48-agent coding pipeline.\n\n\
                 Usage: /archon-code <task description>\n\n\
                 Example: /archon-code implement a REST API for user management\n"
                    .to_string(),
            ));
            return Ok(());
        }

        let task = args.join(" ");
        let coding: Arc<CodingFacade> = match ctx.coding_pipeline.clone() {
            Some(f) => f,
            None => {
                ctx.emit(TuiEvent::Error(
                    "Coding pipeline not available (no LLM configured).".into(),
                ));
                return Ok(());
            }
        };

        let llm: Arc<dyn LlmClient> = match ctx.llm_adapter.clone() {
            Some(l) => l,
            None => {
                ctx.emit(TuiEvent::Error(
                    "LLM adapter not available (no auth configured).".into(),
                ));
                return Ok(());
            }
        };

        let tui_tx = ctx.tui_tx.clone();
        let leann = ctx.leann.clone();

        // Facade emits per-agent progress as Strings; forward to TUI as TextDelta.
        let (string_tx, mut string_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        coding.set_tui_sender(string_tx);
        let fwd_tx = tui_tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = string_rx.recv().await {
                let _ = fwd_tx.send(TuiEvent::TextDelta(msg));
            }
        });

        let _ = tui_tx.send(TuiEvent::TextDelta(format!(
            "Starting coding pipeline for task: {task}\n",
        )));

        tokio::spawn(async move {
            match run_pipeline(
                coding.as_ref(),
                llm.as_ref(),
                &task,
                leann.as_deref(),
                None,
                None,
            )
            .await
            {
                Ok(result) => {
                    let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                        "\n=== Pipeline Complete ===\n\
                         Session: {}\n\
                         Agents run: {}\n\
                         Total cost: ${:.4}\n\
                         Duration: {:.1}s\n",
                        result.session_id,
                        result.agent_results.len(),
                        result.total_cost_usd,
                        result.duration.as_secs_f64(),
                    )));
                }
                Err(e) => {
                    let _ = tui_tx.send(TuiEvent::Error(format!("Coding pipeline failed: {e}")));
                }
            }
        });

        Ok(())
    }

    fn description(&self) -> &str {
        "Run the 48-agent coding pipeline on a task"
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
    fn archon_code_handler_no_args_emits_usage() {
        let (mut ctx, mut rx) = make_ctx();
        ArchonCodeHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let has_usage = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Usage:")));
        assert!(has_usage, "no-args must emit usage text, got: {:?}", events);
    }

    #[test]
    fn archon_code_handler_description_matches() {
        let desc = ArchonCodeHandler.description();
        assert!(
            desc.contains("48-agent"),
            "description must mention 48-agent, got: {desc}"
        );
    }

    #[test]
    fn archon_code_handler_no_pipeline_emits_error() {
        let (mut ctx, mut rx) = make_ctx();
        ArchonCodeHandler
            .execute(&mut ctx, &["test".into(), "task".into()])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_error,
            "missing pipeline must emit error, got: {:?}",
            events
        );
    }
}
