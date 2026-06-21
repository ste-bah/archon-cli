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
        bundle: value.bundle.map(from_bundle),
        approval: value.approval.map(from_approval),
        harness: value.harness,
        compiled_spec: value.compiled_spec,
        stages: value.stages.into_iter().map(from_stage).collect(),
        agents: value.agents.into_iter().map(from_agent).collect(),
        v2_results: value.v2_results.into_iter().map(from_v2_result).collect(),
        v2_branches: value.v2_branches.into_iter().map(from_v2_branch).collect(),
        artifacts: value.artifacts.into_iter().map(from_artifact).collect(),
        events: value.events.into_iter().map(from_event).collect(),
    }
}

fn from_approval(value: archon_workflow::web_api::WorkflowApprovalView) -> WorkflowApprovalView {
    WorkflowApprovalView {
        workflow_hash: value.workflow_hash,
        project_root: value.project_root,
        workflow_name: value.workflow_name,
        phase_count: value.phase_count,
        max_agents: value.max_agents,
        max_parallelism: value.max_parallelism,
        write_capable_stages: value.write_capable_stages,
        external_requirements: value.external_requirements,
        cost_warning: value.cost_warning,
        raw_script_path: value.raw_script_path,
        compiled_spec_path: value.compiled_spec_path,
        decision: value.decision,
        decided_at: value.decided_at,
        decided_by: value.decided_by,
    }
}

fn from_bundle(value: archon_workflow::web_api::WorkflowBundleView) -> WorkflowBundleView {
    WorkflowBundleView {
        workflow_path: value.workflow_path,
        compiled_spec_path: value.compiled_spec_path,
        workflow_hash: value.workflow_hash,
        compiled_hash: value.compiled_hash,
        phase_count: value.phase_count,
        max_agents: value.max_agents,
        max_parallelism: value.max_parallelism,
        write_capable_stages: value.write_capable_stages,
    }
}

fn from_agent(value: archon_workflow::web_api::WorkflowAgentView) -> WorkflowAgentView {
    WorkflowAgentView {
        stage_id: value.stage_id,
        item_id: value.item_id,
        status: value.status,
        prompt_path: value.prompt_path,
        input_hash: value.input_hash,
        prompt_hash: value.prompt_hash,
        prompt_created_at: value.prompt_created_at,
        provider: value.provider,
        model: value.model,
        tokens_in: value.tokens_in,
        tokens_out: value.tokens_out,
        cost_usd: value.cost_usd,
        artifact_id: value.artifact_id,
        artifact_path: value.artifact_path,
        result_preview: value.result_preview,
        error: value.error,
        recent_public_tool_calls: value
            .recent_public_tool_calls
            .into_iter()
            .map(from_tool_call)
            .collect(),
        output_path: value.output_path,
    }
}

fn from_v2_result(value: archon_workflow::web_api::WorkflowV2ResultView) -> WorkflowV2ResultView {
    WorkflowV2ResultView {
        call_id: value.call_id,
        status: value.status,
        summary: value.summary,
        result_path: value.result_path,
        artifact_count: value.artifact_count,
        branch_count: value.branch_count,
    }
}

fn from_v2_branch(value: archon_workflow::web_api::WorkflowV2BranchView) -> WorkflowV2BranchView {
    WorkflowV2BranchView {
        call_id: value.call_id,
        item_id: value.item_id,
        role: value.role,
        status: value.status,
        summary: value.summary,
        error: value.error,
        output_path: value.output_path,
    }
}

fn from_tool_call(value: serde_json::Value) -> WorkflowToolCallPreview {
    WorkflowToolCallPreview {
        tool_name: value
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("tool")
            .to_string(),
        input_preview: value.get("input").map(json_preview),
        output_preview: value.get("output").map(json_preview),
    }
}

fn json_preview(value: &serde_json::Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    const LIMIT: usize = 600;
    if text.len() <= LIMIT {
        text
    } else {
        format!("{}...", text.chars().take(LIMIT).collect::<String>())
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
        started_at: value.started_at,
        completed_at: value.completed_at,
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
    if matches!(
        request.action.as_str(),
        "approve-run-once" | "approve-always" | "deny-workflow"
    ) {
        return apply_approval_control(store, request);
    }
    let action = match request.action.as_str() {
        "resume" | "continue" => archon_workflow::LifecycleAction::Resume,
        "repair" => archon_workflow::LifecycleAction::RestartStage(first_repairable_stage(
            store,
            &request.run_id,
        )?),
        "pause" => archon_workflow::LifecycleAction::Pause,
        "cancel" => archon_workflow::LifecycleAction::Cancel,
        "restart-stage" => {
            archon_workflow::LifecycleAction::RestartStage(required_stage(&request)?)
        }
        "restart-item" => archon_workflow::LifecycleAction::RestartItem {
            stage_id: required_stage(&request)?,
            item_id: required_item(&request)?,
        },
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

fn first_repairable_stage(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
) -> archon_workflow::WorkflowResult<String> {
    let run = store.load_state(run_id)?;
    run.spec
        .stages
        .iter()
        .find(|stage| {
            run.stages
                .get(&stage.id)
                .is_some_and(|state| state.status == archon_workflow::StageStatus::Failed)
        })
        .or_else(|| {
            run.spec.stages.iter().find(|stage| {
                run.stages
                    .get(&stage.id)
                    .is_some_and(|state| state.status == archon_workflow::StageStatus::Blocked)
            })
        })
        .map(|stage| stage.id.clone())
        .ok_or_else(|| {
            archon_workflow::WorkflowError::SpecInvalid(format!(
                "workflow {run_id} has no failed or blocked stage to repair"
            ))
        })
}

fn apply_approval_control(
    store: &archon_workflow::WorkflowStore,
    request: WorkflowControlRequest,
) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowRun> {
    let run = store.load_state(&request.run_id)?;
    let approvals = archon_workflow::WorkflowApprovalStore::for_workflow_store(store);
    let project_root = archon_workflow::approval::project_root_from_workflow_root(store.root());
    match request.action.as_str() {
        "approve-run-once" => {
            approvals.approve_run_once(&project_root, store, &run, "web-workbench")?;
        }
        "approve-always" => {
            approvals.approve_always_for_project(&project_root, store, &run, "web-workbench")?;
        }
        "deny-workflow" => {
            approvals.deny_run(&project_root, store, &run, "web-workbench")?;
            return archon_workflow::LifecycleController::new(store.clone())
                .apply(&request.run_id, archon_workflow::LifecycleAction::Cancel);
        }
        other => {
            return Err(archon_workflow::WorkflowError::SpecInvalid(format!(
                "unknown workflow approval action {other}"
            )));
        }
    }
    Ok(run)
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

fn required_item(request: &WorkflowControlRequest) -> archon_workflow::WorkflowResult<String> {
    request
        .item_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            archon_workflow::WorkflowError::SpecInvalid("item_id is required".to_string())
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
        WorkflowBundleView::decl(&cfg),
        WorkflowApprovalView::decl(&cfg),
        WorkflowAgentView::decl(&cfg),
        WorkflowV2ResultView::decl(&cfg),
        WorkflowV2BranchView::decl(&cfg),
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
