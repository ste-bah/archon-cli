//! TUI progress helpers for `/pipeline resume`.

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::audit::types::{BundleState, BundleStatus, PipelineEvent, PipelineEventLine};
use archon_pipeline::runner::PipelineType;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_tui::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};

use crate::command::registry::CommandContext;

pub(super) fn emit_attached_state(ctx: &CommandContext, cwd: &Path, state: &BundleState) {
    let current = state.current_agent_key.as_deref().unwrap_or("<waiting>");
    ctx.emit(TuiEvent::TextDelta(format!(
        "Attached to running {:?} pipeline {}\n\
         Progress: {} completed\n\
         Current agent: {}\n",
        state.pipeline_type, state.session_id, state.completed_agent_count, current
    )));
    if let Some(agent_key) = state.current_agent_key.as_deref() {
        ctx.emit(TuiEvent::AgentActivity(pipeline_activity_update(
            &state.session_id,
            state.completed_agent_count,
            agent_key,
            AgentActivityStatus::Running,
            Some(format!("attached from audit state in {}", cwd.display())),
            None,
        )));
    }
}

pub(super) fn emit_completed_state(ctx: &CommandContext, cwd: &Path, state: &BundleState) {
    let artifact_text = final_artifacts_for_state(cwd, state).unwrap_or_default();
    ctx.emit(TuiEvent::TextDelta(format!(
        "Pipeline {} is already complete.\n\
         Agents run: {}\n\
         Total cost: ${:.4}\n{}",
        state.session_id, state.completed_agent_count, state.total_cost_usd, artifact_text
    )));
}

pub(super) fn spawn_audit_watcher(cwd: PathBuf, session_id: String, tui_tx: TuiEventSender) {
    archon_observability::spawn_named("pipeline-audit-watch", async move {
        let store = PipelineBundleStore::new(&cwd);
        let audit_path = store.bundle_dir(&session_id).join("audit.log");
        let mut offset = std::fs::metadata(&audit_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);

        loop {
            match emit_new_audit_events(&tui_tx, &session_id, &audit_path, &mut offset).await {
                Ok(()) => {}
                Err(error) => {
                    if tui_tx
                        .send_async(TuiEvent::TextDelta(format!(
                            "Pipeline audit watcher paused: {error}\n"
                        )))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }

            match store.load_state(&session_id) {
                Ok(state) if state.status == BundleStatus::Running => {}
                Ok(state) => {
                    if emit_new_audit_events(&tui_tx, &session_id, &audit_path, &mut offset)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let _ = emit_terminal_state(&tui_tx, &cwd, &state).await;
                    break;
                }
                Err(error) => {
                    let _ = tui_tx
                        .send_async(TuiEvent::Error(format!(
                            "Pipeline audit watcher failed: {error}"
                        )))
                        .await;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

async fn emit_new_audit_events(
    tui_tx: &TuiEventSender,
    session_id: &str,
    path: &Path,
    offset: &mut u64,
) -> Result<()> {
    for event in read_new_audit_events(path, offset)? {
        emit_audit_event(tui_tx, session_id, event).await?;
    }
    Ok(())
}

fn read_new_audit_events(path: &Path, offset: &mut u64) -> Result<Vec<PipelineEventLine>> {
    let start = *offset;
    let mut file = OpenOptions::new().read(true).open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut raw = String::new();
    file.read_to_string(&mut raw)?;
    let Some(last_newline) = raw.rfind('\n') else {
        return Ok(Vec::new());
    };
    let complete = &raw[..=last_newline];
    let events = complete
        .lines()
        .map(serde_json::from_str::<PipelineEventLine>)
        .collect::<Result<Vec<_>, _>>()?;
    *offset = start + complete.len() as u64;
    Ok(events)
}

async fn emit_audit_event(
    tui_tx: &TuiEventSender,
    session_id: &str,
    event: PipelineEventLine,
) -> Result<()> {
    let events = match event.event {
        PipelineEvent::AgentPlanned {
            ordinal,
            agent_key,
            phase,
        } => vec![
            TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Running,
                Some(format!("phase {phase} planned")),
                None,
            )),
            TuiEvent::TextDelta(format!("[pipeline phase {phase}] {agent_key} started\n")),
        ],
        PipelineEvent::LlmAttemptStarted {
            ordinal,
            agent_key,
            attempt,
            model,
        } => vec![TuiEvent::AgentActivity(pipeline_activity_update(
            session_id,
            ordinal,
            &agent_key,
            AgentActivityStatus::Running,
            Some(format!("LLM attempt {attempt} running")),
            Some(model),
        ))],
        PipelineEvent::AgentRetried {
            ordinal,
            agent_key,
            attempt,
            reason,
        } => vec![TuiEvent::AgentActivity(pipeline_activity_update(
            session_id,
            ordinal,
            &agent_key,
            AgentActivityStatus::Running,
            Some(format!("retry {attempt}: {reason}")),
            None,
        ))],
        PipelineEvent::QualityGateForceAccepted {
            ordinal,
            agent_key,
            attempt,
            overall,
            threshold,
            reason,
        } => vec![
            TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Running,
                Some(format!(
                    "force accepted attempt {attempt}: score {overall:.2}/{threshold:.2}"
                )),
                None,
            )),
            TuiEvent::TextDelta(format!(
                "[pipeline] {agent_key} quality gate force-accepted: {reason}\n"
            )),
        ],
        PipelineEvent::LlmAttemptFailed {
            ordinal,
            agent_key,
            attempt,
            error,
        } => vec![
            TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Failed,
                Some(format!("attempt {attempt} failed: {error}")),
                None,
            )),
            TuiEvent::TextDelta(format!(
                "[pipeline] {agent_key} attempt {attempt} failed: {error}\n"
            )),
        ],
        PipelineEvent::AgentCompleted {
            ordinal, agent_key, ..
        } => vec![
            TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Complete,
                Some("complete".to_string()),
                None,
            )),
            TuiEvent::TextDelta(format!("[pipeline] {agent_key} complete\n")),
        ],
        PipelineEvent::ArtifactWritten {
            artifact_type,
            path,
            ..
        } if artifact_type.contains("research-paper") => vec![TuiEvent::TextDelta(format!(
            "[pipeline artifact] {artifact_type}: {path}\n"
        ))],
        PipelineEvent::RunFailed { error } => {
            vec![TuiEvent::Error(format!("Pipeline failed: {error}"))]
        }
        PipelineEvent::RunCompleted {
            completed_agent_count,
            ..
        } => vec![TuiEvent::TextDelta(format!(
            "Pipeline complete: {completed_agent_count} agents completed.\n"
        ))],
        _ => Vec::new(),
    };
    for event in events {
        tui_tx.send_async(event).await?;
    }
    Ok(())
}

async fn emit_terminal_state(
    tui_tx: &TuiEventSender,
    cwd: &Path,
    state: &BundleState,
) -> Result<()> {
    let event = match state.status {
        BundleStatus::Completed => {
            let artifact_text = final_artifacts_for_state(cwd, state).unwrap_or_default();
            Some(TuiEvent::TextDelta(format!(
                "Pipeline {} is already complete.\n\
                 Agents run: {}\n\
                 Total cost: ${:.4}\n{}",
                state.session_id, state.completed_agent_count, state.total_cost_usd, artifact_text
            )))
        }
        BundleStatus::Failed => {
            let detail = state.last_error.as_deref().unwrap_or("unknown error");
            Some(TuiEvent::Error(format!(
                "Pipeline {} failed: {detail}",
                state.session_id
            )))
        }
        BundleStatus::Aborted => Some(TuiEvent::TextDelta(format!(
            "Pipeline {} was aborted.\n",
            state.session_id
        ))),
        BundleStatus::Running => None,
    };
    if let Some(event) = event {
        tui_tx.send_async(event).await?;
    }
    Ok(())
}

fn final_artifacts_for_state(cwd: &Path, state: &BundleState) -> Option<String> {
    if state.pipeline_type != PipelineType::Research {
        return None;
    }
    let bundle_dir = PipelineBundleStore::new(cwd).bundle_dir(&state.session_id);
    let (markdown, pdf) = archon_pipeline::research::final_artifact::artifact_paths(&bundle_dir);
    if markdown.exists() || pdf.exists() {
        Some(format!(
            "Final paper Markdown: {}\nFinal paper PDF: {}\n",
            markdown.display(),
            pdf.display()
        ))
    } else {
        None
    }
}

fn pipeline_activity_update(
    session_id: &str,
    ordinal: usize,
    agent_key: &str,
    status: AgentActivityStatus,
    detail: Option<String>,
    model: Option<String>,
) -> AgentActivityUpdate {
    AgentActivityUpdate {
        id: format!("pipeline:{session_id}:{ordinal}:{agent_key}"),
        name: agent_key.to_string(),
        role: AgentActivityRole::Subagent,
        status,
        current_tool: None,
        detail,
        run_id: Some(session_id.to_string()),
        parent_id: Some(format!("pipeline:{session_id}")),
        artifact_id: None,
        provider: None,
        model,
        cost_usd: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_audit_line_is_retried_after_append_completes_it() {
        let temp = tempfile::tempdir().unwrap();
        let audit_path = temp.path().join("audit.log");
        let line = serde_json::to_string(&PipelineEventLine {
            ts: chrono::Utc::now(),
            event: PipelineEvent::RunCompleted {
                final_output_hash: "hash".into(),
                completed_agent_count: 3,
            },
        })
        .unwrap();
        let split = line.len() / 2;
        std::fs::write(&audit_path, &line[..split]).unwrap();
        let mut offset = 0;

        assert!(
            read_new_audit_events(&audit_path, &mut offset)
                .unwrap()
                .is_empty()
        );
        assert_eq!(offset, 0, "partial trailing line must remain unread");

        use std::io::Write;
        let mut file = OpenOptions::new().append(true).open(&audit_path).unwrap();
        writeln!(file, "{}", &line[split..]).unwrap();

        let events = read_new_audit_events(&audit_path, &mut offset).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event,
            PipelineEvent::RunCompleted {
                completed_agent_count: 3,
                ..
            }
        ));
        assert_eq!(offset, std::fs::metadata(audit_path).unwrap().len());
    }

    #[tokio::test]
    async fn final_drain_emits_terminal_audit_event() {
        let temp = tempfile::tempdir().unwrap();
        let audit_path = temp.path().join("audit.log");
        let line = PipelineEventLine {
            ts: chrono::Utc::now(),
            event: PipelineEvent::RunCompleted {
                final_output_hash: "hash".into(),
                completed_agent_count: 3,
            },
        };
        std::fs::write(
            &audit_path,
            format!("{}\n", serde_json::to_string(&line).unwrap()),
        )
        .unwrap();
        let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
        let mut offset = 0;

        emit_new_audit_events(&tx, "session-1", &audit_path, &mut offset)
            .await
            .expect("final drain");

        assert!(matches!(
            rx.recv().await,
            Some(TuiEvent::TextDelta(text))
                if text == "Pipeline complete: 3 agents completed.\n"
        ));
        assert_eq!(offset, std::fs::metadata(audit_path).unwrap().len());
    }

    #[tokio::test]
    async fn audit_event_waits_for_capacity_and_preserves_order() {
        let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
        tx.send(TuiEvent::GenerationStarted).expect("fill queue");
        let emitter = tokio::spawn(async move {
            emit_audit_event(
                &tx,
                "session-1",
                PipelineEventLine {
                    ts: chrono::Utc::now(),
                    event: PipelineEvent::AgentPlanned {
                        ordinal: 1,
                        agent_key: "contract-agent".into(),
                        phase: 1,
                    },
                },
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(!emitter.is_finished());

        assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
        assert!(matches!(
            rx.recv().await,
            Some(TuiEvent::AgentActivity(update)) if update.name == "contract-agent"
        ));
        emitter.await.expect("emitter task").expect("emit event");
        assert!(matches!(
            rx.recv().await,
            Some(TuiEvent::TextDelta(text)) if text == "[pipeline phase 1] contract-agent started\n"
        ));
    }

    #[test]
    fn activity_update_is_stable_per_pipeline_agent() {
        let update = pipeline_activity_update(
            "session-1",
            7,
            "method-designer",
            AgentActivityStatus::Running,
            Some("running".to_string()),
            Some("gpt-5.4".to_string()),
        );

        assert_eq!(update.id, "pipeline:session-1:7:method-designer");
        assert_eq!(update.name, "method-designer");
        assert_eq!(update.role, AgentActivityRole::Subagent);
        assert_eq!(update.run_id.as_deref(), Some("session-1"));
        assert_eq!(update.model.as_deref(), Some("gpt-5.4"));
    }
}
