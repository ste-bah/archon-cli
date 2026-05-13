use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth};

const MAX_MESSAGE_CHARS: usize = 32_768;
const MAX_ATTACHMENTS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebChatAttachment {
    pub file_name: String,
    pub size_bytes: u64,
    pub mime_type: String,
    pub accepted: bool,
    pub policy_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebChatSubmitRequest {
    pub message: String,
    pub attachments: Vec<WebChatAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebChatSubmitResponse {
    pub message_id: String,
    pub accepted: bool,
    pub created_at_ms: u128,
    pub policy_reason: String,
    pub stored_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WebChatLedgerRow {
    message_id: String,
    message: String,
    attachments: Vec<WebChatAttachment>,
    created_at_ms: u128,
}

pub(crate) async fn submit_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebChatSubmitRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let response = evaluate_chat_submit(&request);
    if response.accepted
        && let Err(err) = append_chat_row(&request, &response)
    {
        tracing::warn!("web chat submit append failed: {err}");
    }
    (StatusCode::OK, Json(response)).into_response()
}

pub fn evaluate_chat_submit(request: &WebChatSubmitRequest) -> WebChatSubmitResponse {
    let message = request.message.trim();
    let created_at_ms = now_ms();
    let stored_path = chat_ledger_path()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();

    if message.is_empty() && request.attachments.is_empty() {
        return response(
            false,
            "chat submit denied: message and attachments are empty",
            stored_path,
        );
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return response(
            false,
            "chat submit denied: message exceeds 32768 characters",
            stored_path,
        );
    }
    if request.attachments.len() > MAX_ATTACHMENTS {
        return response(
            false,
            "chat submit denied: too many attachments",
            stored_path,
        );
    }
    if request
        .attachments
        .iter()
        .any(|attachment| !attachment.accepted)
    {
        return response(
            false,
            "chat submit denied: one or more attachments failed policy",
            stored_path,
        );
    }

    WebChatSubmitResponse {
        message_id: format!("webmsg_{}", uuid::Uuid::new_v4()),
        accepted: true,
        created_at_ms,
        policy_reason: "chat message accepted and recorded by the web workbench".into(),
        stored_path,
    }
}

fn response(accepted: bool, reason: &str, stored_path: String) -> WebChatSubmitResponse {
    WebChatSubmitResponse {
        message_id: String::new(),
        accepted,
        created_at_ms: now_ms(),
        policy_reason: reason.into(),
        stored_path,
    }
}

fn append_chat_row(
    request: &WebChatSubmitRequest,
    response: &WebChatSubmitResponse,
) -> anyhow::Result<()> {
    let Some(path) = chat_ledger_path() else {
        anyhow::bail!("home directory unavailable");
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let row = WebChatLedgerRow {
        message_id: response.message_id.clone(),
        message: request.message.clone(),
        attachments: request.attachments.clone(),
        created_at_ms: response.created_at_ms,
    };
    let mut line = serde_json::to_string(&row)?;
    line.push('\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))?;
    Ok(())
}

fn chat_ledger_path() -> Option<std::path::PathBuf> {
    Some(
        dirs::home_dir()?
            .join(".archon")
            .join("web")
            .join("chat.messages.jsonl"),
    )
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
        exported(WebChatAttachment::decl(&cfg)),
        exported(WebChatSubmitRequest::decl(&cfg)),
        exported(WebChatSubmitResponse::decl(&cfg)),
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
    fn chat_submit_rejects_empty_payload() {
        let response = evaluate_chat_submit(&WebChatSubmitRequest {
            message: "  ".into(),
            attachments: Vec::new(),
        });
        assert!(!response.accepted);
    }

    #[test]
    fn chat_submit_accepts_text_message() {
        let response = evaluate_chat_submit(&WebChatSubmitRequest {
            message: "hello".into(),
            attachments: Vec::new(),
        });
        assert!(response.accepted);
        assert!(response.message_id.starts_with("webmsg_"));
    }

    #[test]
    fn chat_submit_rejects_denied_attachment() {
        let response = evaluate_chat_submit(&WebChatSubmitRequest {
            message: String::new(),
            attachments: vec![WebChatAttachment {
                file_name: "secret.bin".into(),
                size_bytes: 42,
                mime_type: "application/octet-stream".into(),
                accepted: false,
                policy_reason: "denied".into(),
            }],
        });
        assert!(!response.accepted);
    }
}
