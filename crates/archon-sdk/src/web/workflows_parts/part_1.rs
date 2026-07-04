use std::{convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;
use ts_rs::TS;

use super::{
    AppState,
    actions::{WebActionRequest, evaluate_action},
    check_auth,
};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowRunDetail {
    pub summary: WorkflowRunSummary,
    pub bundle: Option<WorkflowBundleView>,
    pub approval: Option<WorkflowApprovalView>,
    pub harness: Option<String>,
    pub compiled_spec: Option<String>,
    pub stages: Vec<WorkflowStageView>,
    pub agents: Vec<WorkflowAgentView>,
    pub v2_results: Vec<WorkflowV2ResultView>,
    pub v2_branches: Vec<WorkflowV2BranchView>,
    pub artifacts: Vec<WorkflowArtifactView>,
    pub events: Vec<WorkflowEventPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowStageView {
    pub id: String,
    pub status: String,
    pub attempt: u32,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub artifacts: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowArtifactView {
    pub id: String,
    pub path: String,
    pub producing_stage: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowBundleView {
    pub workflow_path: String,
    pub compiled_spec_path: String,
    pub workflow_hash: String,
    pub compiled_hash: String,
    pub phase_count: usize,
    pub max_agents: u32,
    pub max_parallelism: u32,
    pub write_capable_stages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowApprovalView {
    pub workflow_hash: String,
    pub project_root: String,
    pub workflow_name: String,
    pub phase_count: usize,
    pub max_agents: u32,
    pub max_parallelism: u32,
    pub write_capable_stages: Vec<String>,
    pub external_requirements: Vec<String>,
    pub cost_warning: String,
    pub raw_script_path: String,
    pub compiled_spec_path: String,
    pub decision: Option<String>,
    pub decided_at: Option<String>,
    pub decided_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowAgentView {
    pub stage_id: String,
    pub item_id: String,
    pub status: String,
    pub prompt_path: Option<String>,
    pub input_hash: Option<String>,
    pub prompt_hash: Option<String>,
    pub prompt_created_at: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub artifact_id: Option<String>,
    pub artifact_path: Option<String>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub recent_public_tool_calls: Vec<WorkflowToolCallPreview>,
    pub output_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowV2ResultView {
    pub call_id: String,
    pub status: String,
    pub summary: String,
    pub result_path: String,
    pub artifact_count: usize,
    pub branch_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowV2BranchView {
    pub call_id: String,
    pub item_id: String,
    pub role: String,
    pub status: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub output_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowToolCallPreview {
    pub tool_name: String,
    pub input_preview: Option<String>,
    pub output_preview: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowControlRequest {
    pub run_id: String,
    pub action: String,
    pub stage_id: Option<String>,
    #[serde(default)]
    pub item_id: Option<String>,
    pub rationale: Option<String>,
    pub confirmation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowControlResponse {
    pub allowed: bool,
    pub policy_reason: String,
    pub run: Option<WorkflowRunSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowEventQuery {
    pub after: Option<u64>,
    pub limit: Option<usize>,
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

pub(crate) async fn detail_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let store = archon_workflow::WorkflowStore::project(&state.paths.cwd);
    match archon_workflow::web_api::detail(&store, &run_id) {
        Ok(detail) => (StatusCode::OK, Json(from_detail(detail))).into_response(),
        Err(error) => (
            StatusCode::NOT_FOUND,
            format!("workflow detail failed: {error}"),
        )
            .into_response(),
    }
}

pub(crate) async fn events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Query(query): Query<WorkflowEventQuery>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let store = archon_workflow::WorkflowStore::project(&state.paths.cwd);
    let limit = query.limit.unwrap_or(100).min(500);
    match archon_workflow::web_api::event_previews_after(
        &store,
        &run_id,
        query.after.unwrap_or(0),
        limit,
    ) {
        Ok(events) => (
            StatusCode::OK,
            Json(events.into_iter().map(from_event).collect::<Vec<_>>()),
        )
            .into_response(),
        Err(error) => (
            StatusCode::NOT_FOUND,
            format!("workflow events failed: {error}"),
        )
            .into_response(),
    }
}

pub(crate) async fn stream_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Query(query): Query<WorkflowEventQuery>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let store = archon_workflow::WorkflowStore::project(&state.paths.cwd);
    let mut after = query.after.unwrap_or(0);
    let stream =
        tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
            .map(move |_| {
                let events =
                    archon_workflow::web_api::event_previews_after(&store, &run_id, after, 100)
                        .unwrap_or_default();
                after = events.iter().map(|event| event.seq).max().unwrap_or(after);
                Ok::<_, Infallible>(sse_event(events.into_iter().map(from_event).collect()))
            });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

pub(crate) async fn control_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WorkflowControlRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let decision = workflow_action_decision(&state, &request);
    if !decision.allowed {
        return (StatusCode::FORBIDDEN, Json(decision)).into_response();
    }
    let store = archon_workflow::WorkflowStore::project(&state.paths.cwd);
    match apply_control(&store, request) {
        Ok(run) => (
            StatusCode::OK,
            Json(WorkflowControlResponse {
                allowed: true,
                policy_reason: "allowed by workflow web control policy".to_string(),
                run: Some(from_run(
                    archon_workflow::web_api::WorkflowRunSummary::from(&run),
                )),
            }),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(WorkflowControlResponse {
                allowed: false,
                policy_reason: error.to_string(),
                run: None,
            }),
        )
            .into_response(),
    }
}
