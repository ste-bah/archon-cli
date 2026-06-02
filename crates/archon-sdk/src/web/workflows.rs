use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{AppState, check_auth};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowWebSummary {
    pub root: String,
    pub runs: Vec<WorkflowRunSummary>,
    pub events: Vec<WorkflowEventPreview>,
    pub controls: Vec<WorkflowControlPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    pub id: String,
    pub name: String,
    pub status: String,
    pub stage_count: usize,
    pub accepted_count: usize,
    pub failed_count: usize,
    pub artifact_count: usize,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowEventPreview {
    pub run_id: String,
    pub seq: u64,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowControlPreview {
    pub action: String,
    pub enabled: bool,
    pub policy_reason: String,
}

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let store = archon_workflow::WorkflowStore::project(&state.paths.cwd);
    match archon_workflow::web_api::summary(&store, 24) {
        Ok(summary) => (StatusCode::OK, Json(from_workflow_summary(summary))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("workflow summary failed: {error}"),
        )
            .into_response(),
    }
}

fn from_workflow_summary(
    value: archon_workflow::web_api::WorkflowWebSummary,
) -> WorkflowWebSummary {
    WorkflowWebSummary {
        root: value.root,
        runs: value.runs.into_iter().map(from_run).collect(),
        events: value.events.into_iter().map(from_event).collect(),
        controls: value.controls.into_iter().map(from_control).collect(),
    }
}

fn from_run(value: archon_workflow::web_api::WorkflowRunSummary) -> WorkflowRunSummary {
    WorkflowRunSummary {
        id: value.id,
        name: value.name,
        status: format!("{:?}", value.status).to_ascii_lowercase(),
        stage_count: value.stage_count,
        accepted_count: value.accepted_count,
        failed_count: value.failed_count,
        artifact_count: value.artifact_count,
        updated_at: value.updated_at,
    }
}

fn from_event(value: archon_workflow::web_api::WorkflowEventPreview) -> WorkflowEventPreview {
    WorkflowEventPreview {
        run_id: value.run_id,
        seq: value.seq,
        kind: format!("{:?}", value.kind).to_ascii_lowercase(),
        status: value.status,
        summary: value.summary,
        created_at: value.created_at,
    }
}

fn from_control(value: archon_workflow::web_api::WorkflowControlPreview) -> WorkflowControlPreview {
    WorkflowControlPreview {
        action: value.action,
        enabled: value.enabled,
        policy_reason: value.policy_reason,
    }
}

pub fn generated_typescript() -> String {
    let cfg = ts_rs::Config::default().with_large_int("number");
    [
        WorkflowWebSummary::decl(&cfg),
        WorkflowRunSummary::decl(&cfg),
        WorkflowEventPreview::decl(&cfg),
        WorkflowControlPreview::decl(&cfg),
    ]
    .into_iter()
    .map(|decl| format!("export {decl}"))
    .collect::<Vec<_>>()
    .join("\n\n")
}
