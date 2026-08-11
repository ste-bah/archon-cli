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
    pub index_queue: WebIndexQueueSummary,
    pub index_jobs: Vec<WebIndexJobItem>,
    pub index_failures: Vec<WebIndexFailureItem>,
    pub warnings: Vec<String>,
    /// Why the knowledge-base listing may be short, if it is.
    ///
    /// Kept separate from `warnings` so the tab that renders the list can say
    /// so in place. An unreadable store and a store with no knowledge bases
    /// both produce an empty list, and they must not look the same.
    pub knowledge_base_warnings: Vec<String>,
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
    /// The exact string to pass to `--kb`. Where a directory slug and a stored
    /// `kb_id` differ this is the `kb_id`, because that is what `--kb` matches.
    pub name: String,
    pub scope: String,
    pub path: String,
    pub files: u64,
    pub bytes: u64,
    pub exists: bool,
    /// Where this knowledge base is recorded: `db`, `dir`, or `both`. Shown so
    /// a split is visible, never used to decide whether to list it.
    pub origin: String,
    /// Documents attached in the store. Always zero for a directory-only
    /// knowledge base, which has no membership rows.
    pub documents: u64,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIndexQueueSummary {
    pub pending: u64,
    pub leased: u64,
    pub indexed: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIndexJobItem {
    pub job_id: String,
    pub status: String,
    pub scope: String,
    pub provider: String,
    pub leased: i64,
    pub indexed: i64,
    pub failed: i64,
    pub skipped: i64,
    pub started_at: String,
    pub last_error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIndexFailureItem {
    pub chunk_id: String,
    pub document_id: String,
    pub attempt_count: i64,
    pub last_error: String,
    pub updated_at: String,
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

/// Status for an action policy refused.
///
/// Both mutating handlers here used to answer `200 OK` with `accepted: false`,
/// so a caller that checked only the status read a denial as a success and
/// waited for a knowledge base or an ingest job that was never going to
/// arrive (#170). The body still carries the decision and its reason; the
/// status is what makes the refusal impossible to miss.
const DENIED_STATUS: StatusCode = StatusCode::FORBIDDEN;

pub(crate) async fn summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let jobs = state.ingest_jobs.lock().await.clone();
    let paths = state.paths.clone();
    let policy = state.api.policy();
    match run_summary_blocking(move || summary(&paths, &policy, jobs)).await {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("ingest summary blocking task failed: {error}"),
        )
            .into_response(),
    }
}

async fn run_summary_blocking<T>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, tokio::task::JoinError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work).await
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
            DENIED_STATUS,
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
            DENIED_STATUS,
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
        exported(WebIndexQueueSummary::decl(&cfg)),
        exported(WebIndexJobItem::decl(&cfg)),
        exported(WebIndexFailureItem::decl(&cfg)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn summary_blocking_work_yields_to_the_async_runtime() {
        let (release, blocked) = std::sync::mpsc::channel();
        let work = tokio::spawn(run_summary_blocking(move || {
            blocked.recv().unwrap();
            1
        }));

        tokio::task::yield_now().await;
        assert_eq!(tokio::spawn(async { 2 }).await.unwrap(), 2);
        release.send(()).unwrap();
        assert_eq!(work.await.unwrap().unwrap(), 1);
    }

    /// A refusal has to be visible to a caller that only looks at the status
    /// line. Both mutating handlers in this module answered `200 OK` with
    /// `accepted: false`, which reads as "done" to anything that does not
    /// unpack the body.
    #[test]
    fn a_policy_refusal_does_not_answer_with_a_success_status() {
        assert!(
            !DENIED_STATUS.is_success(),
            "a denial must not carry a 2xx status, got {DENIED_STATUS}"
        );
        assert_eq!(DENIED_STATUS, StatusCode::FORBIDDEN);
    }

    /// The status is the loud part; the body still has to explain why, or the
    /// refusal is untraceable.
    #[test]
    fn a_refused_create_still_carries_its_reason() {
        let policy = denying_policy();
        let decision = ingest_decision(&policy, true);
        assert!(!decision.allowed);
        let response = WebKbCreateResponse {
            accepted: false,
            decision,
            knowledge_base: None,
        };
        assert!(!response.accepted);
        assert!(
            response.decision.policy_reason.contains("denied"),
            "unexpected reason: {}",
            response.decision.policy_reason
        );
    }

    fn denying_policy() -> EffectivePolicySummary {
        EffectivePolicySummary::default_safe()
    }
}
