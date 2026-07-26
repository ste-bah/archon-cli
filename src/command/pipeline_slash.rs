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
    LlmClient, PipelineFacade, PipelineProgressFacade, PipelineResult, PipelineRunOptions,
    PipelineType, resume_pipeline_audited_with_options,
};
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;

use crate::command::pipeline_slash_progress::{
    emit_attached_state, emit_completed_state, spawn_audit_watcher,
};
use crate::command::pipeline_support::{
    build_interactive_learning_stack, build_reflexion_injector, final_research_artifact_paths,
};
use crate::command::registry::{CommandContext, CommandHandler, PipelineWork};

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
            emit_attached_state(ctx, &cwd, &state);
            ctx.pending_effect = Some(crate::command::registry::CommandEffect::StartPipelineWork(
                PipelineWork::Watch {
                    cwd,
                    session_id: session_id.to_string(),
                },
            ));
            Ok(())
        }
        BundleStatus::Completed => {
            emit_completed_state(ctx, &cwd, &state);
            Ok(())
        }
        BundleStatus::Failed | BundleStatus::Aborted => {
            ctx.pending_effect = Some(crate::command::registry::CommandEffect::StartPipelineWork(
                PipelineWork::Resume {
                    cwd,
                    pipeline_type: state.pipeline_type,
                    session_id: session_id.to_string(),
                    force_quality_gate,
                },
            ));
            Ok(())
        }
    }
}

pub(crate) async fn start_pipeline_work(
    ctx: &crate::slash_context::SlashCommandContext,
    tui_tx: &TuiEventSender,
    work: PipelineWork,
) {
    match work {
        PipelineWork::Watch { cwd, session_id } => {
            spawn_audit_watcher(cwd, session_id, tui_tx.clone());
        }
        PipelineWork::Resume {
            cwd,
            pipeline_type,
            session_id,
            force_quality_gate,
        } => {
            let _ = resume_in_process(
                ctx,
                tui_tx,
                cwd,
                pipeline_type,
                session_id,
                force_quality_gate,
            )
            .await;
        }
    }
}

async fn resume_in_process(
    ctx: &crate::slash_context::SlashCommandContext,
    tui_tx: &TuiEventSender,
    cwd: PathBuf,
    pipeline_type: PipelineType,
    session_id: String,
    force_quality_gate: bool,
) -> Result<()> {
    let llm: Arc<dyn LlmClient> = Arc::clone(&ctx.llm_adapter);
    let loaded_config = archon_core::config::load_config().ok();
    let mut learning = loaded_config.as_ref().and_then(|config| {
        build_interactive_learning_stack(config, ctx.cozo_db.clone(), ctx.auto_trainer.clone())
    });
    let mut reflexion = loaded_config.as_ref().and_then(build_reflexion_injector);
    let tui_tx = tui_tx.clone();
    let options = PipelineRunOptions { force_quality_gate };

    match pipeline_type {
        PipelineType::Coding => {
            let coding: Arc<dyn PipelineFacade> = Arc::clone(&ctx.coding_pipeline) as _;
            let (progress_facade, progress_forwarder) =
                attach_progress_forwarder("pipeline-resume-code-progress", coding, tui_tx.clone());
            let leann = ctx.leann.clone();
            let session = session_id.clone();
            tui_tx
                .send_async(TuiEvent::TextDelta(format!(
                    "{}\n",
                    resume_status_line("coding", &session, force_quality_gate)
                )))
                .await?;
            archon_observability::spawn_named("pipeline-resume-code", async move {
                let result = resume_pipeline_audited_with_options(
                    &progress_facade,
                    llm.as_ref(),
                    &session,
                    &cwd,
                    leann.as_deref(),
                    (reflexion.as_mut(), learning.as_mut()),
                    options,
                )
                .await;
                drop(progress_facade);
                if let Err(error) = progress_forwarder.await {
                    tracing::error!(%error, "coding resume progress forwarder failed");
                }
                emit_resume_result(&tui_tx, &cwd, result).await;
            });
        }
        PipelineType::Research => {
            let research: Arc<dyn PipelineFacade> = Arc::clone(&ctx.research_pipeline) as _;
            let (progress_facade, progress_forwarder) = attach_progress_forwarder(
                "pipeline-resume-research-progress",
                research,
                tui_tx.clone(),
            );
            let session = session_id.clone();
            tui_tx
                .send_async(TuiEvent::TextDelta(format!(
                    "{}\n",
                    resume_status_line("research", &session, force_quality_gate)
                )))
                .await?;
            archon_observability::spawn_named("pipeline-resume-research", async move {
                let result = resume_pipeline_audited_with_options(
                    &progress_facade,
                    llm.as_ref(),
                    &session,
                    &cwd,
                    None,
                    (reflexion.as_mut(), learning.as_mut()),
                    options,
                )
                .await;
                drop(progress_facade);
                if let Err(error) = progress_forwarder.await {
                    tracing::error!(%error, "research resume progress forwarder failed");
                }
                emit_resume_result(&tui_tx, &cwd, result).await;
            });
        }
        other => {
            tui_tx
                .send_async(TuiEvent::Error(format!(
                    "Unsupported audited pipeline type for TUI resume: {other:?}"
                )))
                .await?;
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

fn attach_progress_forwarder(
    name: &'static str,
    facade: Arc<dyn PipelineFacade>,
    tui_tx: TuiEventSender,
) -> (PipelineProgressFacade, tokio::task::JoinHandle<()>) {
    let (string_tx, mut string_rx) = tokio::sync::mpsc::channel::<String>(1);
    let progress_facade = PipelineProgressFacade::new(facade, string_tx);
    let forwarder = archon_observability::spawn_named(name, async move {
        while let Some(msg) = string_rx.recv().await {
            if tui_tx.send_async(TuiEvent::TextDelta(msg)).await.is_err() {
                return;
            }
        }
    });
    (progress_facade, forwarder)
}

async fn emit_resume_result(tui_tx: &TuiEventSender, cwd: &Path, result: Result<PipelineResult>) {
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
            let _ = tui_tx
                .send_async(TuiEvent::TextDelta(format!(
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
                )))
                .await;
        }
        Err(error) => {
            let _ = tui_tx
                .send_async(TuiEvent::Error(format!("Pipeline resume failed: {error}")))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::{CtxBuilder, drain_tui_events};

    #[test]
    fn slash_dispatch_starts_pipeline_work_only_after_successful_flush() {
        let source = include_str!("slash.rs");
        let flush = source.find("flush_events().await").expect("flush call");
        let gate = source
            .find("events_flushed\n                || !matches!")
            .expect("successful flush gate");
        let apply = source
            .find("apply_effect(effect, ctx, tui_tx).await")
            .expect("effect application");
        assert!(flush < gate && gate < apply);
    }

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
        assert!(matches!(
            ctx.pending_effect,
            Some(crate::command::registry::CommandEffect::StartPipelineWork(
                _
            ))
        ));

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
