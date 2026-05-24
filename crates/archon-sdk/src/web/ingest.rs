use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{
    AppState,
    api::{EffectivePolicySummary, WebActionDecision},
    check_auth,
    ingest_jobs::{command_args, start_job},
    ingest_store::{create_kb, summary},
    inspect::PathProbe,
};

pub(crate) use super::ingest_jobs::{WebIngestJobStore, new_job_store};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIngestSummary {
    pub allowed: bool,
    pub policy_reason: String,
    pub stores: Vec<PathProbe>,
    pub documents: Vec<WebDocStoreItem>,
    pub videos: Vec<WebVideoStoreItem>,
    pub knowledge_bases: Vec<WebKnowledgeBaseItem>,
    pub kb_stats: WebKnowledgeStats,
    pub jobs: Vec<WebIngestJob>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebDocStoreItem {
    pub document_id: String,
    pub source_path: String,
    pub media_type: String,
    pub status: String,
    pub chunks: u64,
    pub pages: u64,
    pub artifacts: u64,
    pub ocr_runs: u64,
    pub discovered_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebVideoStoreItem {
    pub video_id: String,
    pub document_id: String,
    pub title: String,
    pub source: String,
    pub status: String,
    pub duration_ms: i64,
    pub chunks: u64,
    pub transcript_segments: u64,
    pub frames: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebKnowledgeBaseItem {
    pub name: String,
    pub scope: String,
    pub path: String,
    pub files: u64,
    pub bytes: u64,
    pub exists: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebKnowledgeStats {
    pub chunks: u64,
    pub claims: u64,
    pub entities: u64,
    pub relations: u64,
    pub contradictions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIngestJob {
    pub job_id: String,
    pub label: String,
    pub target: String,
    pub command: String,
    pub status: String,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIngestRunRequest {
    pub target: String,
    pub source: String,
    pub frames: Option<String>,
    pub asr: Option<String>,
    pub transcript: Option<String>,
    pub vlm: bool,
    pub metadata_only: bool,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIngestRunResponse {
    pub accepted: bool,
    pub decision: WebActionDecision,
    pub job: Option<WebIngestJob>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebKbCreateRequest {
    pub name: String,
    pub scope: String,
    pub description: Option<String>,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebKbCreateResponse {
    pub accepted: bool,
    pub decision: WebActionDecision,
    pub knowledge_base: Option<WebKnowledgeBaseItem>,
}

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let jobs = state.ingest_jobs.lock().await.clone();
    (
        StatusCode::OK,
        Json(summary(&state.paths, &state.api.policy(), jobs)),
    )
        .into_response()
}

pub(crate) async fn run_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebIngestRunRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let decision = ingest_decision(&state.api.policy(), request.confirmed);
    if !decision.allowed {
        return (
            StatusCode::OK,
            Json(WebIngestRunResponse {
                accepted: false,
                decision,
                job: None,
            }),
        )
            .into_response();
    }
    let args = match command_args(&request) {
        Ok(args) => args,
        Err(reason) => {
            return (StatusCode::BAD_REQUEST, reason).into_response();
        }
    };
    let job = start_job(&state, request.target.clone(), request.source.clone(), args).await;
    (
        StatusCode::OK,
        Json(WebIngestRunResponse {
            accepted: true,
            decision,
            job: Some(job),
        }),
    )
        .into_response()
}

pub(crate) async fn create_kb_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebKbCreateRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let decision = ingest_decision(&state.api.policy(), request.confirmed);
    if !decision.allowed {
        return (
            StatusCode::OK,
            Json(WebKbCreateResponse {
                accepted: false,
                decision,
                knowledge_base: None,
            }),
        )
            .into_response();
    }
    match create_kb(&state.paths, &request) {
        Ok(item) => (
            StatusCode::OK,
            Json(WebKbCreateResponse {
                accepted: true,
                decision,
                knowledge_base: Some(item),
            }),
        )
            .into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

fn ingest_decision(policy: &EffectivePolicySummary, confirmed: bool) -> WebActionDecision {
    let (allowed, reason) = ingest_allowed(policy);
    WebActionDecision {
        allowed: allowed && confirmed,
        requires_confirmation: true,
        policy_reason: if !allowed {
            reason
        } else if confirmed {
            "ingest allowed by web mutation and upload policy".into()
        } else {
            "confirmation required before starting ingest".into()
        },
        dry_run_available: true,
    }
}

pub(crate) fn ingest_allowed(policy: &EffectivePolicySummary) -> (bool, String) {
    if !policy.web.allow_mutating_actions {
        return (
            false,
            "denied: policy.web.allow_mutating_actions is false".into(),
        );
    }
    if !policy.web.allow_file_uploads || !policy.subsystem.allow_file_uploads {
        return (
            false,
            "denied: web file upload/ingest policy is disabled".into(),
        );
    }
    (true, "web ingest actions allowed by policy".into())
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebIngestSummary::decl(&cfg)),
        exported(WebDocStoreItem::decl(&cfg)),
        exported(WebVideoStoreItem::decl(&cfg)),
        exported(WebKnowledgeBaseItem::decl(&cfg)),
        exported(WebKnowledgeStats::decl(&cfg)),
        exported(WebIngestJob::decl(&cfg)),
        exported(WebIngestRunRequest::decl(&cfg)),
        exported(WebIngestRunResponse::decl(&cfg)),
        exported(WebKbCreateRequest::decl(&cfg)),
        exported(WebKbCreateResponse::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}
