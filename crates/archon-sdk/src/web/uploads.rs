use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, api::WebActionDecision, check_auth};

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
    (StatusCode::OK, Json(default_policy())).into_response()
}

pub(crate) async fn intent_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(_intent): Json<WebUploadIntent>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let response = WebUploadIntentResponse {
        accepted: false,
        decision: WebActionDecision {
            allowed: false,
            requires_confirmation: false,
            policy_reason: "file uploads are disabled until web upload policy is connected"
                .to_string(),
            dry_run_available: true,
        },
    };
    (StatusCode::OK, Json(response)).into_response()
}

pub fn default_policy() -> WebUploadPolicy {
    WebUploadPolicy {
        enabled: false,
        max_files: 0,
        max_bytes_per_file: 0,
        accepted_mime_types: Vec::new(),
        policy_reason: "file uploads are disabled by default".to_string(),
    }
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
        let policy = default_policy();
        assert!(!policy.enabled);
        assert_eq!(policy.max_files, 0);
    }
}
