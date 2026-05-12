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
};

const MAX_FILES: u16 = 8;
const MAX_BYTES_PER_FILE: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebUploadPolicy {
    pub enabled: bool,
    pub max_files: u16,
    pub max_bytes_per_file: u64,
    pub accepted_mime_types: Vec<String>,
    pub policy_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebUploadIntent {
    pub file_name: String,
    pub size_bytes: u64,
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebUploadIntentResponse {
    pub decision: WebActionDecision,
    pub accepted: bool,
}

pub(crate) async fn policy_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let policy = state.api.policy();
    (StatusCode::OK, Json(upload_policy(&policy))).into_response()
}

pub(crate) async fn intent_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(intent): Json<WebUploadIntent>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let policy = state.api.policy();
    let upload_policy = upload_policy(&policy);
    let decision = upload_intent_decision(&upload_policy, &intent);
    let response = WebUploadIntentResponse {
        accepted: decision.allowed,
        decision,
    };
    (StatusCode::OK, Json(response)).into_response()
}

pub fn upload_policy(policy: &EffectivePolicySummary) -> WebUploadPolicy {
    let enabled = policy.web.allow_file_uploads
        && policy.web.allow_mutating_actions
        && policy.subsystem.allow_file_uploads;
    WebUploadPolicy {
        enabled,
        max_files: if enabled { MAX_FILES } else { 0 },
        max_bytes_per_file: if enabled { MAX_BYTES_PER_FILE } else { 0 },
        accepted_mime_types: if enabled {
            accepted_mime_types()
        } else {
            Vec::new()
        },
        policy_reason: if enabled {
            "file uploads allowed by web and upload subsystem policy".to_string()
        } else if !policy.web.allow_mutating_actions {
            "file uploads denied: policy.web.allow_mutating_actions is false".to_string()
        } else if !policy.web.allow_file_uploads {
            "file uploads denied: policy.web.allow_file_uploads is false".to_string()
        } else {
            "file uploads denied: upload subsystem policy is false".to_string()
        },
    }
}

fn upload_intent_decision(policy: &WebUploadPolicy, intent: &WebUploadIntent) -> WebActionDecision {
    if !policy.enabled {
        return denied(policy.policy_reason.clone());
    }
    if intent.size_bytes > policy.max_bytes_per_file {
        return denied(format!(
            "file is too large: {} bytes exceeds {}",
            intent.size_bytes, policy.max_bytes_per_file
        ));
    }
    if !policy
        .accepted_mime_types
        .iter()
        .any(|accepted| accepted == &intent.mime_type)
    {
        return denied(format!(
            "MIME type '{}' is not accepted by web upload policy",
            intent.mime_type
        ));
    }
    WebActionDecision {
        allowed: true,
        requires_confirmation: false,
        policy_reason: "upload intent accepted by web upload policy".to_string(),
        dry_run_available: true,
    }
}

fn denied(reason: impl Into<String>) -> WebActionDecision {
    WebActionDecision {
        allowed: false,
        requires_confirmation: false,
        policy_reason: reason.into(),
        dry_run_available: true,
    }
}

fn accepted_mime_types() -> Vec<String> {
    [
        "text/plain",
        "text/markdown",
        "application/pdf",
        "application/json",
        "application/x-yaml",
        "image/png",
        "image/jpeg",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebUploadPolicy::decl(&cfg)),
        exported(WebUploadIntent::decl(&cfg)),
        exported(WebUploadIntentResponse::decl(&cfg)),
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

    #[test]
    fn upload_policy_is_default_deny() {
        let policy = upload_policy(&EffectivePolicySummary::default_safe());
        assert!(!policy.enabled);
        assert_eq!(policy.max_files, 0);
    }

    #[test]
    fn upload_policy_allows_valid_intent_when_gates_allow() {
        let mut effective = EffectivePolicySummary::default_safe();
        effective.web.allow_mutating_actions = true;
        effective.web.allow_file_uploads = true;
        effective.subsystem.allow_file_uploads = true;
        let policy = upload_policy(&effective);
        let decision = upload_intent_decision(
            &policy,
            &WebUploadIntent {
                file_name: "notes.md".to_string(),
                size_bytes: 512,
                mime_type: "text/markdown".to_string(),
            },
        );
        assert!(decision.allowed);
    }
}
