//! `/pipeline` slash-command handler.
//!
//! Most `/pipeline ...` subcommands continue to mirror the CLI. `resume` is
//! handled in-process so the TUI can attach to live audited bundles and route
//! resumed subagents through the active session activity sink.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::audit::types::BundleStatus;
use archon_pipeline::runner::{
    LlmClient, PipelineResult, PipelineRunOptions, PipelineType,
    resume_pipeline_audited_with_options,
};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;

use crate::command::pipeline_slash_progress::{
    emit_attached_state, emit_completed_state, spawn_audit_watcher,
};
use crate::command::pipeline_support::{
    build_interactive_learning_stack, build_reflexion_injector, final_research_artifact_paths,
};
use crate::command::registry::{CommandContext, CommandHandler};

/// TUI-aware `/pipeline` umbrella.
pub(crate) struct PipelineSlashHandler;

impl CommandHandler for PipelineSlashHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        match args {
            [] => emit_pipeline_usage(ctx),
            [subcommand, session_id] if subcommand == "resume" => {
                handle_tui_resume(ctx, session_id, false)
            }
            [subcommand, session_id, flag]
                if subcommand == "resume" && flag == "--force-quality-gate" =>
            {
                handle_tui_resume(ctx, session_id, true)
            }
            [subcommand, ..] if subcommand == "resume" => {
                ctx.emit(TuiEvent::TextDelta(
                    "Usage: /pipeline resume <session-id> [--force-quality-gate]\n".to_string(),
                ));
                Ok(())
            }
            _ => crate::command::cli_mirror::spawn_cli_mirror(ctx, "pipeline", args),
        }
    }

    fn description(&self) -> &str {
        "Run pipeline commands from inside the TUI"
    }
}

fn emit_pipeline_usage(ctx: &mut CommandContext) -> Result<()> {
    ctx.emit(TuiEvent::TextDelta(
        "Usage: /pipeline <subcommand> [args]\n\
         TUI-native: /pipeline resume <session-id> [--force-quality-gate]\n\
         Other subcommands mirror `archon pipeline ...`.\n"
            .to_string(),
    ));
    Ok(())
}

fn handle_tui_resume(
    ctx: &mut CommandContext,
    session_id: &str,
    force_quality_gate: bool,
) -> Result<()> {
    let cwd = ctx
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let store = PipelineBundleStore::new(&cwd);
    let state = match store.load_state(session_id) {
        Ok(state) => state,
        Err(_) => {
            let mut mirror_args = vec!["resume".to_string(), session_id.to_string()];
            if force_quality_gate {
                mirror_args.push("--force-quality-gate".to_string());
            }
            return crate::command::cli_mirror::spawn_cli_mirror(ctx, "pipeline", &mirror_args);
        }
    };

    match state.status {
        BundleStatus::Running => {
            emit_attached_state(&ctx.tui_tx, &cwd, &state);
            spawn_audit_watcher(cwd, session_id.to_string(), ctx.tui_tx.clone());
            Ok(())
        }
        BundleStatus::Completed => {
            emit_completed_state(&ctx.tui_tx, &cwd, &state);
            Ok(())
        }
        BundleStatus::Failed | BundleStatus::Aborted => resume_in_process(
            ctx,
            cwd,
            state.pipeline_type,
            session_id.to_string(),
            force_quality_gate,
        ),
    }
}

fn resume_in_process(
    ctx: &mut CommandContext,
    cwd: PathBuf,
    pipeline_type: PipelineType,
    session_id: String,
    force_quality_gate: bool,
) -> Result<()> {
    let llm: Arc<dyn LlmClient> = match ctx.llm_adapter.clone() {
        Some(llm) => llm,
        None => {
            ctx.emit(TuiEvent::Error(
                "LLM adapter not available; cannot resume pipeline in the TUI.".into(),
            ));
            return Ok(());
        }
    };
    let loaded_config = archon_core::config::load_config().ok();
    let mut learning = loaded_config.as_ref().and_then(|config| {
        build_interactive_learning_stack(config, ctx.cozo_db.clone(), ctx.auto_trainer.clone())
    });
    let mut reflexion = loaded_config.as_ref().and_then(build_reflexion_injector);
    let tui_tx = ctx.tui_tx.clone();
    let options = PipelineRunOptions { force_quality_gate };

    match pipeline_type {
        PipelineType::Coding => {
            let coding = match ctx.coding_pipeline.clone() {
                Some(coding) => coding,
                None => {
                    ctx.emit(TuiEvent::Error(
                        "Coding pipeline facade not available.".into(),
                    ));
                    return Ok(());
                }
            };
            attach_progress_forwarder("pipeline-resume-code-progress", &coding, tui_tx.clone());
            let leann = ctx.leann.clone();
            let session = session_id.clone();
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "{}\n",
                resume_status_line("coding", &session, force_quality_gate)
            )));
            archon_observability::spawn_named("pipeline-resume-code", async move {
                let result = resume_pipeline_audited_with_options(
                    coding.as_ref(),
                    llm.as_ref(),
                    &session,
                    &cwd,
                    leann.as_deref(),
                    reflexion.as_mut(),
                    learning.as_mut(),
                    options,
                )
                .await;
                emit_resume_result(&tui_tx, &cwd, result);
            });
        }
        PipelineType::Research => {
            let research = match ctx.research_pipeline.clone() {
                Some(research) => research,
                None => {
                    ctx.emit(TuiEvent::Error(
                        "Research pipeline facade not available.".into(),
                    ));
                    return Ok(());
                }
            };
            attach_progress_forwarder(
                "pipeline-resume-research-progress",
                &research,
                tui_tx.clone(),
            );
            let session = session_id.clone();
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "{}\n",
                resume_status_line("research", &session, force_quality_gate)
            )));
            archon_observability::spawn_named("pipeline-resume-research", async move {
                let result = resume_pipeline_audited_with_options(
                    research.as_ref(),
                    llm.as_ref(),
                    &session,
                    &cwd,
                    None,
                    reflexion.as_mut(),
                    learning.as_mut(),
                    options,
                )
                .await;
                emit_resume_result(&tui_tx, &cwd, result);
            });
        }
        other => {
            ctx.emit(TuiEvent::Error(format!(
                "Unsupported audited pipeline type for TUI resume: {other:?}"
            )));
        }
    }
    Ok(())
}

fn resume_status_line(kind: &str, session_id: &str, force_quality_gate: bool) -> String {
    if force_quality_gate {
        format!(
            "Resuming {kind} pipeline {session_id} in the TUI with audited quality-gate override..."
        )
    } else {
        format!("Resuming {kind} pipeline {session_id} in the TUI...")
    }
}

trait TuiProgressFacade {
    fn set_progress_sender(&self, tx: tokio::sync::mpsc::UnboundedSender<String>);
}

impl TuiProgressFacade for archon_pipeline::coding::facade::CodingFacade {
    fn set_progress_sender(&self, tx: tokio::sync::mpsc::UnboundedSender<String>) {
        self.set_tui_sender(tx);
    }
}

impl TuiProgressFacade for archon_pipeline::research::facade::ResearchFacade {
    fn set_progress_sender(&self, tx: tokio::sync::mpsc::UnboundedSender<String>) {
        self.set_tui_sender(tx);
    }
}

fn attach_progress_forwarder<F>(name: &'static str, facade: &Arc<F>, tui_tx: TuiEventSender)
where
    F: TuiProgressFacade + ?Sized + Send + Sync + 'static,
{
    let (string_tx, mut string_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    facade.set_progress_sender(string_tx);
    archon_observability::spawn_named(name, async move {
        while let Some(msg) = string_rx.recv().await {
            let _ = tui_tx.send(TuiEvent::TextDelta(msg));
        }
    });
}

fn emit_resume_result(tui_tx: &TuiEventSender, cwd: &Path, result: Result<PipelineResult>) {
    match result {
        Ok(result) => {
            let artifacts = final_research_artifact_paths(&result, cwd)
                .map(|(markdown, pdf)| {
                    format!(
                        "Final paper Markdown: {}\nFinal paper PDF: {}\n",
                        markdown.display(),
                        pdf.display()
                    )
                })
                .unwrap_or_default();
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "\n=== Pipeline Complete ===\n\
                 Session: {}\n\
                 Agents run: {}\n\
                 Total cost: ${:.4}\n\
                 Duration: {:.1}s\n{}",
                result.session_id,
                result.agent_results.len(),
                result.total_cost_usd,
                result.duration.as_secs_f64(),
                artifacts,
            )));
        }
        Err(error) => {
            let _ = tui_tx.send(TuiEvent::Error(format!("Pipeline resume failed: {error}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::{CtxBuilder, drain_tui_events};

    #[tokio::test]
    async fn running_resume_attaches_without_appending_run_resumed() {
        let temp = tempfile::tempdir().unwrap();
        let store = PipelineBundleStore::new(temp.path());
        let state = store
            .create(
                "session-1",
                PipelineType::Research,
                "research task needing resume visibility",
            )
            .unwrap();
        assert_eq!(state.status, BundleStatus::Running);
        let before = std::fs::read_to_string(store.bundle_dir("session-1").join("audit.log"))
            .unwrap()
            .matches("\"run_resumed\"")
            .count();

        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(temp.path().to_path_buf())
            .build();
        PipelineSlashHandler
            .execute(&mut ctx, &["resume".to_string(), "session-1".to_string()])
            .unwrap();

        let events = drain_tui_events(&mut rx);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TuiEvent::TextDelta(text) if text.contains("Attached to running Research pipeline session-1"))),
            "expected attach progress, got {events:?}",
        );
        let after = std::fs::read_to_string(store.bundle_dir("session-1").join("audit.log"))
            .unwrap()
            .matches("\"run_resumed\"")
            .count();
        assert_eq!(before, after, "running attach must not resume again");
    }

    #[test]
    fn completed_resume_reports_artifacts_without_spawning_cli() {
        let temp = tempfile::tempdir().unwrap();
        let store = PipelineBundleStore::new(temp.path());
        let mut state = store
            .create("session-2", PipelineType::Coding, "coding task")
            .unwrap();
        state.status = BundleStatus::Completed;
        state.completed_agent_count = 3;
        store.save_state(&state).unwrap();

        let (mut ctx, mut rx) = CtxBuilder::new()
            .with_working_dir(temp.path().to_path_buf())
            .build();
        PipelineSlashHandler
            .execute(&mut ctx, &["resume".to_string(), "session-2".to_string()])
            .unwrap();

        let events = drain_tui_events(&mut rx);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TuiEvent::TextDelta(text) if text.contains("already complete"))),
            "expected completed status, got {events:?}",
        );
    }
}
