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
    pub stages: Vec<WorkflowStageView>,
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

#[derive(Debug, Clone, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WorkflowControlRequest {
    pub run_id: String,
    pub action: String,
    pub stage_id: Option<String>,
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

fn from_detail(value: archon_workflow::web_api::WorkflowRunDetail) -> WorkflowRunDetail {
    WorkflowRunDetail {
        summary: from_run(value.summary),
        stages: value.stages.into_iter().map(from_stage).collect(),
        artifacts: value.artifacts.into_iter().map(from_artifact).collect(),
        events: value.events.into_iter().map(from_event).collect(),
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

fn from_stage(value: archon_workflow::web_api::WorkflowStageView) -> WorkflowStageView {
    WorkflowStageView {
        id: value.id,
        status: format!("{:?}", value.status).to_ascii_lowercase(),
        attempt: value.attempt,
        artifacts: value.artifacts,
        error: value.error,
    }
}

fn from_artifact(value: archon_workflow::web_api::WorkflowArtifactView) -> WorkflowArtifactView {
    WorkflowArtifactView {
        id: value.id,
        path: value.path,
        producing_stage: value.producing_stage,
        content_hash: value.content_hash,
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

fn workflow_action_decision(
    state: &AppState,
    request: &WorkflowControlRequest,
) -> WorkflowControlResponse {
    let action = WebActionRequest {
        action_id: format!("workflow:{}:{}", request.run_id, request.action),
        action_kind: format!("pipeline.workflow.{}", request.action),
        dry_run: false,
        payload_summary: request.run_id.clone(),
        confirmation_token: request.confirmation_token.clone(),
    };
    let decision = evaluate_action(action, &state.api.policy()).decision;
    WorkflowControlResponse {
        allowed: decision.allowed,
        policy_reason: decision.policy_reason,
        run: None,
    }
}

fn apply_control(
    store: &archon_workflow::WorkflowStore,
    request: WorkflowControlRequest,
) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowRun> {
    let action = match request.action.as_str() {
        "resume" => archon_workflow::LifecycleAction::Resume,
        "pause" => archon_workflow::LifecycleAction::Pause,
        "cancel" => archon_workflow::LifecycleAction::Cancel,
        "restart-stage" => {
            archon_workflow::LifecycleAction::RestartStage(required_stage(&request)?)
        }
        "force-accept" => archon_workflow::LifecycleAction::ForceAcceptStage {
            stage_id: required_stage(&request)?,
            forced_by: "web-workbench".to_string(),
            rationale: required_rationale(&request)?,
            source: "web".to_string(),
        },
        other => {
            return Err(archon_workflow::WorkflowError::SpecInvalid(format!(
                "unknown workflow control action {other}"
            )));
        }
    };
    archon_workflow::LifecycleController::new(store.clone()).apply(&request.run_id, action)
}

fn required_stage(request: &WorkflowControlRequest) -> archon_workflow::WorkflowResult<String> {
    request
        .stage_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            archon_workflow::WorkflowError::SpecInvalid("stage_id is required".to_string())
        })
}

fn required_rationale(request: &WorkflowControlRequest) -> archon_workflow::WorkflowResult<String> {
    request
        .rationale
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            archon_workflow::WorkflowError::SpecInvalid("rationale is required".to_string())
        })
}

fn sse_event(events: Vec<WorkflowEventPreview>) -> Event {
    Event::default()
        .event("workflow-events")
        .json_data(events)
        .unwrap_or_else(|_| {
            Event::default()
                .event("workflow-error")
                .data("serialization failed")
        })
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
        WorkflowRunDetail::decl(&cfg),
        WorkflowStageView::decl(&cfg),
        WorkflowArtifactView::decl(&cfg),
        WorkflowControlRequest::decl(&cfg),
        WorkflowControlResponse::decl(&cfg),
    ]
    .into_iter()
    .map(|decl| format!("export {}", decl.trim_end()))
    .collect::<Vec<_>>()
    .join("\n\n")
}
