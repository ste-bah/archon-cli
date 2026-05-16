//! `/archon-research` slash-command handler.
//!
//! Spawns the 46-agent PhD research pipeline via `tokio::spawn` wrapping
//! `archon_pipeline::runner::run_pipeline_audited()`. Per-agent progress events
//! are streamed to the TUI via the facade's `tui_sender`.

use std::sync::Arc;

use crate::command::registry::{CommandContext, CommandHandler};
use archon_pipeline::research::facade::ResearchFacade;
use archon_pipeline::runner::{LlmClient, run_pipeline_audited};
use archon_tui::app::TuiEvent;

/// Handler for `/archon-research <topic>`.
pub(crate) struct ArchonResearchHandler;

impl CommandHandler for ArchonResearchHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        if args.is_empty() {
            ctx.emit(TuiEvent::TextDelta(
                "\n/archon-research — run the 46-agent PhD research pipeline.\n\n\
                 Usage: /archon-research <research topic>\n\n\
                 Example: /archon-research impact of transformer architectures on NLP\n"
                    .to_string(),
            ));
            return Ok(());
        }

        let topic = args.join(" ");
        let research: Arc<ResearchFacade> = match ctx.research_pipeline.clone() {
            Some(f) => f,
            None => {
                ctx.emit(TuiEvent::Error(
                    "Research pipeline not available (no memory backend).".into(),
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
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        // Facade emits per-agent progress as Strings; forward to TUI as TextDelta.
        let (string_tx, mut string_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        research.set_tui_sender(string_tx);
        let fwd_tx = tui_tx.clone();
        archon_observability::spawn_named("archon-research-progress-forwarder", async move {
            while let Some(msg) = string_rx.recv().await {
                let _ = fwd_tx.send(TuiEvent::TextDelta(msg));
            }
        });

        let _ = tui_tx.send(TuiEvent::TextDelta(format!(
            "Starting research pipeline for topic: {topic}\n",
        )));
        let world_context = archon_core::config::load_config().ok().map(|config| {
            let guardrail = crate::command::world_model::begin_guarded_action(
                &config,
                archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
                "archon-research",
                "tui_archon_research_start",
                &format!("research pipeline: {topic}"),
            );
            let advisory = guardrail
                .as_ref()
                .map(|record| record.advisory.clone())
                .unwrap_or_else(|| {
                    crate::command::world_model::record_runtime_advisory(
                        &config,
                        archon_world_model::integration::WorldAdvisorSurface::Pipeline,
                        "archon-research",
                        "tui_archon_research_start",
                        &format!("research pipeline: {topic}"),
                    )
                });
            tracing::debug!(
                continue_foreground_flow = advisory.continue_foreground_flow,
                "world_model.tui_archon_research_advisory"
            );
            (config, guardrail, advisory)
        });
        if let Some((config, guardrail, _)) = world_context.as_ref() {
            if let Some(record) = guardrail
                && !record.decision.allowed_to_finalize
                && !record.decision.required_actions.is_empty()
            {
                let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                    "World model guardrail: {:?} risk; pipeline completion requires {:?}.\n",
                    record.decision.risk_tier, record.decision.required_actions
                )));
            }
            let _ = crate::command::world_model::record_runtime_counterfactual_advice(
                config,
                archon_world_model::integration::WorldAdvisorSurface::PipelineStep,
                &topic,
                &[
                    ("pipeline-research", "run the full research pipeline now"),
                    ("verify-first", "run source verification before research"),
                    ("resume-existing", "resume a previous research pipeline"),
                    (
                        "surface-memory",
                        "surface relevant memories before research",
                    ),
                    (
                        "provider-fallback",
                        "switch provider before pipeline execution",
                    ),
                ],
            );
        }

        archon_observability::spawn_named("archon-research-pipeline", async move {
            match run_pipeline_audited(
                research.as_ref(),
                llm.as_ref(),
                &topic,
                &cwd,
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
                    if let Some((config, guardrail, advisory)) = world_context.as_ref() {
                        if let Some(record) = guardrail {
                            let step_report =
                                crate::command::world_model::record_guardrail_pipeline_steps(
                                    config, record, &result,
                                );
                            if step_report.steps_recorded > 0 {
                                let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                                    "World model guardrail: recorded {} pipeline steps and {} verification signals.\n",
                                    step_report.steps_recorded,
                                    step_report.parent_verifications_recorded
                                )));
                            }
                            if let Some(outcome) =
                                crate::command::world_model::record_guardrail_completion_outcome(
                                    config,
                                    record,
                                    true,
                                    &result.final_output,
                                    Some(&result.session_id),
                                )
                                && matches!(
                                    outcome.final_status,
                                    archon_world_model::GuardrailFinalStatus::BlockedMissingVerification
                                        | archon_world_model::GuardrailFinalStatus::BlockedFailedVerification
                                )
                            {
                                let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                                    "World model guardrail: pipeline output is not marked verified yet; required actions: {:?}\n",
                                    record.decision.required_actions
                                )));
                            }
                        } else {
                            crate::command::world_model::record_runtime_outcome(
                                config,
                                advisory,
                                &result.final_output,
                                Some(&result.session_id),
                            );
                        }
                        crate::command::world_model::schedule_dynamic_trainer_tick(config.clone());
                    }
                }
                Err(e) => {
                    let _ = tui_tx.send(TuiEvent::Error(format!("Research pipeline failed: {e}")));
                }
            }
        });

        Ok(())
    }

    fn description(&self) -> &str {
        "Run the 46-agent PhD research pipeline on a topic"
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
        archon_tui::event_channel::TuiEventReceiver,
    ) {
        CtxBuilder::new().build()
    }

    #[test]
    fn archon_research_handler_no_args_emits_usage() {
        let (mut ctx, mut rx) = make_ctx();
        ArchonResearchHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let has_usage = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s.contains("Usage:")));
        assert!(has_usage, "no-args must emit usage text, got: {:?}", events);
    }

    #[test]
    fn archon_research_handler_description_matches() {
        let desc = ArchonResearchHandler.description();
        assert!(
            desc.contains("46-agent"),
            "description must mention 46-agent, got: {desc}"
        );
    }

    #[test]
    fn archon_research_handler_no_pipeline_emits_error() {
        let (mut ctx, mut rx) = make_ctx();
        ArchonResearchHandler
            .execute(&mut ctx, &["test".into(), "topic".into()])
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
