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
pub struct WebActionRequest {
    pub action_id: String,
    pub action_kind: String,
    pub dry_run: bool,
    pub payload_summary: String,
    pub confirmation_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebActionAuditRow {
    pub action_id: String,
    pub action_kind: String,
    pub allowed: bool,
    pub dry_run: bool,
    pub policy_reason: String,
    pub created_at_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebActionResponse {
    pub decision: WebActionDecision,
    pub audit: WebActionAuditRow,
}

pub(crate) async fn evaluate_action_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebActionRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let response = evaluate_action(request);
    if let Err(err) = append_audit_row(&response.audit) {
        tracing::warn!("web action audit append failed: {err}");
    }
    (StatusCode::OK, Json(response)).into_response()
}

pub fn evaluate_action(request: WebActionRequest) -> WebActionResponse {
    let decision = WebActionDecision {
        allowed: false,
        requires_confirmation: true,
        policy_reason: "web mutating actions are disabled until subsystem policy gates are wired"
            .to_string(),
        dry_run_available: true,
    };
    let audit = WebActionAuditRow {
        action_id: request.action_id,
        action_kind: request.action_kind,
        allowed: decision.allowed,
        dry_run: request.dry_run,
        policy_reason: decision.policy_reason.clone(),
        created_at_ms: now_ms(),
    };
    WebActionResponse { decision, audit }
}

fn append_audit_row(row: &WebActionAuditRow) -> anyhow::Result<()> {
    let Some(home) = dirs::home_dir() else {
        anyhow::bail!("home directory unavailable");
    };
    let dir = home.join(".archon").join("web");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("actions.audit.jsonl");
    let mut line = serde_json::to_string(row)?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))?;
    Ok(())
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebActionRequest::decl(&cfg)),
        exported(WebActionAuditRow::decl(&cfg)),
        exported(WebActionResponse::decl(&cfg)),
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
    fn action_envelope_is_default_deny() {
        let response = evaluate_action(WebActionRequest {
            action_id: "act_1".to_string(),
            action_kind: "pipeline.pause".to_string(),
            dry_run: true,
            payload_summary: "pause run".to_string(),
            confirmation_token: None,
        });
        assert!(!response.decision.allowed);
        assert!(response.decision.requires_confirmation);
        assert_eq!(response.audit.action_kind, "pipeline.pause");
    }
}
