//! Document deletion and semantic-index control.
//!
//! The two operations the Ingest tab could display but not perform. The queue
//! counters were already on the page; the verbs that move them were only in the
//! CLI, and a document could be ingested from the browser but never removed.
//!
//! Both go straight to the library rather than shelling out to `archon`.
//! Deletion in particular must call [`archon_docs::delete::delete_document`]
//! and not reimplement a subset of it — see [`delete_handler`].

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{
    AppState, api::EffectivePolicySummary, check_auth, ingest::ingest_allowed,
    ingest_store::open_docs_db,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebDocDeleteRequest {
    pub document_id: String,
    /// The browser has shown the operator what will be removed and they said
    /// yes. Without it the request is a dry run that reports the counts.
    pub confirmed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebDocDeleteResponse {
    pub accepted: bool,
    pub policy_reason: String,
    pub document_id: String,
    pub source_path: String,
    pub chunks: u64,
    pub pages: u64,
    pub artifacts: u64,
    pub vectors: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIndexControlRequest {
    /// `pause`, `resume`, `cancel` or `retryFailed`.
    pub action: String,
    /// Required by every action except `retryFailed`.
    pub job_id: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebIndexControlResponse {
    pub accepted: bool,
    pub policy_reason: String,
    pub detail: String,
}

/// Delete a document and everything derived from it.
///
/// The handler is thin on purpose. `delete_document` reuses `reprocess`'s
/// teardown and additionally drops the registration rows, and **dropping
/// `doc_sources` is the load-bearing part**: content-hash dedupe reads
/// `doc_sources.content_hash` directly, so a delete that clears chunks and
/// leaves the source row makes the document vanish from every view while
/// re-ingesting the identical bytes still reports `Skipped: 1 duplicates`
/// forever. That is strictly worse than no delete at all, which is why nothing
/// here reimplements any part of the cascade.
pub(crate) async fn delete_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebDocDeleteRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let (allowed, reason) = deletion_allowed(&state.api.policy());
    if !allowed || !request.confirmed {
        return (
            StatusCode::OK,
            Json(WebDocDeleteResponse {
                accepted: false,
                policy_reason: if allowed {
                    "confirmation required before deleting a document".to_string()
                } else {
                    reason
                },
                document_id: request.document_id,
                ..Default::default()
            }),
        )
            .into_response();
    }

    let paths = state.paths.clone();
    let document_id = request.document_id.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let db = open_docs_db(&paths)?;
        archon_docs::delete::delete_document(&db, &document_id).map_err(anyhow::Error::from)
    })
    .await;

    match outcome {
        Ok(Ok(deleted)) => {
            state.live.record("web.docs.deleted", &deleted.source_path);
            (
                StatusCode::OK,
                Json(WebDocDeleteResponse {
                    accepted: true,
                    policy_reason: "document deletion allowed by policy".to_string(),
                    document_id: deleted.document_id,
                    source_path: deleted.source_path,
                    chunks: deleted.chunks as u64,
                    pages: deleted.pages as u64,
                    artifacts: deleted.artifacts as u64,
                    vectors: deleted.vectors as u64,
                }),
            )
                .into_response()
        }
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, format!("{error:#}")).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("delete task failed: {error}"),
        )
            .into_response(),
    }
}

/// Pause, resume, cancel or retry the semantic index queue.
pub(crate) async fn index_control_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebIndexControlRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    // Index control is an ingest-family action: it moves the same queue an
    // ingest fills, so it rides the gate that already governs ingest.
    let (allowed, reason) = ingest_allowed(&state.api.policy());
    if !allowed {
        return (
            StatusCode::OK,
            Json(WebIndexControlResponse {
                accepted: false,
                policy_reason: reason,
                detail: String::new(),
            }),
        )
            .into_response();
    }

    let paths = state.paths.clone();
    let outcome = tokio::task::spawn_blocking(move || apply_index_control(&paths, &request)).await;

    match outcome {
        Ok(Ok(detail)) => {
            state.live.record("web.index.control", &detail);
            (
                StatusCode::OK,
                Json(WebIndexControlResponse {
                    accepted: true,
                    policy_reason: "index control allowed by web ingest policy".to_string(),
                    detail,
                }),
            )
                .into_response()
        }
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, format!("{error:#}")).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("index control task failed: {error}"),
        )
            .into_response(),
    }
}

fn apply_index_control(
    paths: &super::WebRuntimePaths,
    request: &WebIndexControlRequest,
) -> anyhow::Result<String> {
    use archon_docs::{index_jobs, index_queue};

    let db = open_docs_db(paths)?;
    let job_id = || -> anyhow::Result<&str> {
        request
            .job_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("job id is required for '{}'", request.action))
    };

    match request.action.as_str() {
        // Pause and cancel both release the job's leases. A leased chunk whose
        // owner has stopped is otherwise stranded: it is not pending, so no
        // worker picks it up, and not indexed, so nothing reports it missing.
        "pause" => {
            let id = job_id()?;
            index_jobs::pause_job(&db, id)?;
            let released = index_queue::release_leases_for_owner(&db, id)?;
            Ok(format!(
                "paused {id}; released {released} leased chunk(s) back to pending"
            ))
        }
        "resume" => {
            let id = job_id()?;
            index_jobs::resume_job(&db, id)?;
            Ok(format!(
                "{id} marked resumable; run an index pass to drain the queue"
            ))
        }
        "cancel" => {
            let id = job_id()?;
            index_jobs::cancel_job(&db, id)?;
            let released = index_queue::release_leases_for_owner(&db, id)?;
            Ok(format!(
                "cancelled {id}; released {released} leased chunk(s) back to pending"
            ))
        }
        "retryFailed" => {
            let limit = request.limit.map(|value| value as usize);
            let retried = index_queue::retry_failed(&db, limit)?;
            Ok(format!("requeued {retried} failed chunk(s)"))
        }
        other => anyhow::bail!("unsupported index control action: {other}"),
    }
}

/// Deletion needs the global mutation gate **and** its own flag. It is
/// deliberately not implied by `allow_file_uploads`.
pub(crate) fn deletion_allowed(policy: &EffectivePolicySummary) -> (bool, String) {
    if !policy.web.allow_mutating_actions {
        return (
            false,
            "denied: policy.web.allow_mutating_actions is false".to_string(),
        );
    }
    if !policy.web.allow_document_deletion {
        return (
            false,
            "denied: policy.web.allow_document_deletion is false".to_string(),
        );
    }
    (true, "document deletion allowed by policy".to_string())
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebDocDeleteRequest::decl(&cfg)),
        exported(WebDocDeleteResponse::decl(&cfg)),
        exported(WebIndexControlRequest::decl(&cfg)),
        exported(WebIndexControlResponse::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
#[path = "docs_actions_tests.rs"]
mod tests;
