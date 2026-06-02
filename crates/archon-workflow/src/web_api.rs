use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEvent, WorkflowEventKind, contains_forbidden_field};
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowWebSummary {
    pub root: String,
    pub runs: Vec<WorkflowRunSummary>,
    pub events: Vec<WorkflowEventPreview>,
    pub controls: Vec<WorkflowControlPreview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunSummary {
    pub id: String,
    pub name: String,
    pub status: RunStatus,
    pub stage_count: usize,
    pub accepted_count: usize,
    pub failed_count: usize,
    pub artifact_count: usize,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunDetail {
    pub summary: WorkflowRunSummary,
    pub stages: Vec<WorkflowStageView>,
    pub artifacts: Vec<WorkflowArtifactView>,
    pub events: Vec<WorkflowEventPreview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStageView {
    pub id: String,
    pub status: StageStatus,
    pub attempt: u32,
    pub artifacts: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowArtifactView {
    pub id: String,
    pub path: String,
    pub producing_stage: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowEventPreview {
    pub run_id: String,
    pub seq: u64,
    pub kind: WorkflowEventKind,
    pub status: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowControlPreview {
    pub action: String,
    pub enabled: bool,
    pub policy_reason: String,
}

pub fn summary(store: &WorkflowStore, limit: usize) -> WorkflowResult<WorkflowWebSummary> {
    let runs = store
        .list_runs()?
        .into_iter()
        .take(limit)
        .collect::<Vec<_>>();
    let events = runs
        .iter()
        .flat_map(|run| event_previews(store, &run.id, 8).unwrap_or_default())
        .take(limit)
        .collect();
    Ok(WorkflowWebSummary {
        root: store.root().display().to_string(),
        runs: runs.iter().map(WorkflowRunSummary::from).collect(),
        events,
        controls: control_previews(),
    })
}

pub fn detail(store: &WorkflowStore, run_id: &str) -> WorkflowResult<WorkflowRunDetail> {
    let run = store.load_state(run_id)?;
    Ok(WorkflowRunDetail {
        summary: WorkflowRunSummary::from(&run),
        stages: stage_views(&run),
        artifacts: artifact_views(&run),
        events: event_previews(store, run_id, 200)?,
    })
}

pub fn event_previews(
    store: &WorkflowStore,
    run_id: &str,
    limit: usize,
) -> WorkflowResult<Vec<WorkflowEventPreview>> {
    let mut events = read_events(store, run_id)?
        .into_iter()
        .filter(|event| !is_tool_noise(event))
        .map(|event| {
            let status = status_label(&event.kind).to_string();
            let summary = event_summary(&event);
            WorkflowEventPreview {
                run_id: event.run_id,
                seq: event.seq,
                kind: event.kind,
                status,
                summary,
                created_at: event.ts.to_rfc3339(),
            }
        })
        .collect::<Vec<_>>();
    events.sort_by(|a, b| b.seq.cmp(&a.seq));
    events.truncate(limit);
    Ok(events)
}

pub fn event_previews_after(
    store: &WorkflowStore,
    run_id: &str,
    after_seq: u64,
    limit: usize,
) -> WorkflowResult<Vec<WorkflowEventPreview>> {
    let mut events = read_events(store, run_id)?
        .into_iter()
        .filter(|event| event.seq > after_seq)
        .filter(|event| !is_tool_noise(event))
        .map(|event| {
            let status = status_label(&event.kind).to_string();
            let summary = event_summary(&event);
            WorkflowEventPreview {
                run_id: event.run_id,
                seq: event.seq,
                kind: event.kind,
                status,
                summary,
                created_at: event.ts.to_rfc3339(),
            }
        })
        .collect::<Vec<_>>();
    events.sort_by(|a, b| a.seq.cmp(&b.seq));
    events.truncate(limit);
    Ok(events)
}

pub fn artifact_views(run: &WorkflowRun) -> Vec<WorkflowArtifactView> {
    run.stages
        .values()
        .flat_map(|stage| stage.artifacts.iter())
        .map(|artifact| WorkflowArtifactView {
            id: artifact.id.clone(),
            path: artifact.path.display().to_string(),
            producing_stage: artifact.producing_stage.clone(),
            content_hash: artifact.content_hash.clone(),
        })
        .collect()
}

fn read_events(store: &WorkflowStore, run_id: &str) -> WorkflowResult<Vec<WorkflowEvent>> {
    let path = store.events_path(run_id);
    let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
    let mut events = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: WorkflowEvent = serde_json::from_str(line)?;
        if !contains_forbidden_field(&event.detail) {
            events.push(event);
        }
    }
    Ok(events)
}

fn stage_views(run: &WorkflowRun) -> Vec<WorkflowStageView> {
    run.stages
        .values()
        .map(|stage| WorkflowStageView {
            id: stage.id.clone(),
            status: stage.status,
            attempt: stage.attempt,
            artifacts: stage.artifacts.len(),
            error: stage.error.clone(),
        })
        .collect()
}

fn is_tool_noise(event: &WorkflowEvent) -> bool {
    event
        .detail
        .get("raw_tool_output")
        .or_else(|| event.detail.get("tool_stdout"))
        .is_some()
}

fn event_summary(event: &WorkflowEvent) -> String {
    event
        .detail
        .get("stage")
        .or_else(|| event.detail.get("name"))
        .or_else(|| event.detail.get("status"))
        .or_else(|| event.detail.get("error_class"))
        .and_then(|value| value.as_str())
        .unwrap_or("workflow event")
        .to_string()
}

fn status_label(kind: &WorkflowEventKind) -> &'static str {
    match kind {
        WorkflowEventKind::Started | WorkflowEventKind::StageStarted => "running",
        WorkflowEventKind::StageCompleted | WorkflowEventKind::Completed => "completed",
        WorkflowEventKind::StageFailed => "failed",
        WorkflowEventKind::StageSkipped => "skipped",
        WorkflowEventKind::ForcedAccepted => "forced",
        WorkflowEventKind::Resumed => "resumed",
        WorkflowEventKind::Paused => "paused",
        WorkflowEventKind::Cancelled => "cancelled",
        WorkflowEventKind::LearningRecorded => "learning",
    }
}

fn control_previews() -> Vec<WorkflowControlPreview> {
    ["resume", "pause", "cancel", "restart-stage"]
        .into_iter()
        .map(|action| WorkflowControlPreview {
            action: action.to_string(),
            enabled: true,
            policy_reason: "policy gate checked server-side before mutation".to_string(),
        })
        .collect()
}

impl From<&WorkflowRun> for WorkflowRunSummary {
    fn from(run: &WorkflowRun) -> Self {
        let accepted_count = run
            .stages
            .values()
            .filter(|stage| run.accepted_stage(&stage.id))
            .count();
        let failed_count = run
            .stages
            .values()
            .filter(|stage| matches!(stage.status, StageStatus::Failed))
            .count();
        Self {
            id: run.id.clone(),
            name: run.spec.name.clone(),
            status: run.status.clone(),
            stage_count: run.stages.len(),
            accepted_count,
            failed_count,
            artifact_count: artifact_views(run).len(),
            updated_at: run.updated_at.to_rfc3339(),
        }
    }
}
