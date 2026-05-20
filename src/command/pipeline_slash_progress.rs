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

pub(super) fn emit_attached_state(tui_tx: &TuiEventSender, cwd: &Path, state: &BundleState) {
    let current = state.current_agent_key.as_deref().unwrap_or("<waiting>");
    let _ = tui_tx.send(TuiEvent::TextDelta(format!(
        "Attached to running {:?} pipeline {}\n\
         Progress: {} completed\n\
         Current agent: {}\n",
        state.pipeline_type, state.session_id, state.completed_agent_count, current
    )));
    if let Some(agent_key) = state.current_agent_key.as_deref() {
        let _ = tui_tx.send(TuiEvent::AgentActivity(pipeline_activity_update(
            &state.session_id,
            state.completed_agent_count,
            agent_key,
            AgentActivityStatus::Running,
            Some(format!("attached from audit state in {}", cwd.display())),
            None,
        )));
    }
}

pub(super) fn emit_completed_state(tui_tx: &TuiEventSender, cwd: &Path, state: &BundleState) {
    let artifact_text = final_artifacts_for_state(cwd, state).unwrap_or_default();
    let _ = tui_tx.send(TuiEvent::TextDelta(format!(
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
            match read_new_audit_events(&audit_path, &mut offset) {
                Ok(events) => {
                    for event in events {
                        emit_audit_event(&tui_tx, &session_id, event);
                    }
                }
                Err(error) => {
                    let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                        "Pipeline audit watcher paused: {error}\n"
                    )));
                }
            }

            match store.load_state(&session_id) {
                Ok(state) if state.status == BundleStatus::Running => {}
                Ok(state) => {
                    emit_terminal_state(&tui_tx, &cwd, &state);
                    break;
                }
                Err(error) => {
                    let _ = tui_tx.send(TuiEvent::Error(format!(
                        "Pipeline audit watcher failed: {error}"
                    )));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

fn read_new_audit_events(path: &Path, offset: &mut u64) -> Result<Vec<PipelineEventLine>> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    file.seek(SeekFrom::Start(*offset))?;
    let mut raw = String::new();
    file.read_to_string(&mut raw)?;
    *offset = file.stream_position()?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str::<PipelineEventLine>(line).ok())
        .collect())
}

fn emit_audit_event(tui_tx: &TuiEventSender, session_id: &str, event: PipelineEventLine) {
    match event.event {
        PipelineEvent::AgentPlanned {
            ordinal,
            agent_key,
            phase,
        } => {
            let _ = tui_tx.send(TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Running,
                Some(format!("phase {phase} planned")),
                None,
            )));
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "[pipeline phase {phase}] {agent_key} started\n"
            )));
        }
        PipelineEvent::LlmAttemptStarted {
            ordinal,
            agent_key,
            attempt,
            model,
        } => {
            let _ = tui_tx.send(TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Running,
                Some(format!("LLM attempt {attempt} running")),
                Some(model),
            )));
        }
        PipelineEvent::AgentRetried {
            ordinal,
            agent_key,
            attempt,
            reason,
        } => {
            let _ = tui_tx.send(TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Running,
                Some(format!("retry {attempt}: {reason}")),
                None,
            )));
        }
        PipelineEvent::LlmAttemptFailed {
            ordinal,
            agent_key,
            attempt,
            error,
        } => {
            let _ = tui_tx.send(TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Failed,
                Some(format!("attempt {attempt} failed: {error}")),
                None,
            )));
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "[pipeline] {agent_key} attempt {attempt} failed: {error}\n"
            )));
        }
        PipelineEvent::AgentCompleted {
            ordinal, agent_key, ..
        } => {
            let _ = tui_tx.send(TuiEvent::AgentActivity(pipeline_activity_update(
                session_id,
                ordinal,
                &agent_key,
                AgentActivityStatus::Complete,
                Some("complete".to_string()),
                None,
            )));
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "[pipeline] {agent_key} complete\n"
            )));
        }
        PipelineEvent::ArtifactWritten {
            artifact_type,
            path,
            ..
        } if artifact_type.contains("research-paper") => {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "[pipeline artifact] {artifact_type}: {path}\n"
            )));
        }
        PipelineEvent::RunFailed { error } => {
            let _ = tui_tx.send(TuiEvent::Error(format!("Pipeline failed: {error}")));
        }
        PipelineEvent::RunCompleted {
            completed_agent_count,
            ..
        } => {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "Pipeline complete: {completed_agent_count} agents completed.\n"
            )));
        }
        _ => {}
    }
}

fn emit_terminal_state(tui_tx: &TuiEventSender, cwd: &Path, state: &BundleState) {
    match state.status {
        BundleStatus::Completed => emit_completed_state(tui_tx, cwd, state),
        BundleStatus::Failed => {
            let detail = state.last_error.as_deref().unwrap_or("unknown error");
            let _ = tui_tx.send(TuiEvent::Error(format!(
                "Pipeline {} failed: {detail}",
                state.session_id
            )));
        }
        BundleStatus::Aborted => {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "Pipeline {} was aborted.\n",
                state.session_id
            )));
        }
        BundleStatus::Running => {}
    }
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
