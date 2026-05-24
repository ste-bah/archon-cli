use async_trait::async_trait;
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

#[derive(Debug, Clone)]
pub struct WebChatBackendOutput {
    pub reply: String,
    pub policy_reason: String,
    pub attachments: Vec<WebChatAttachment>,
}

#[async_trait]
pub trait WebChatBackend: Send + Sync {
    async fn submit(
        &self,
        message_id: &str,
        request: WebChatSubmitRequest,
    ) -> anyhow::Result<WebChatBackendOutput>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebChatAttachment {
    pub file_name: String,
    pub size_bytes: u64,
    pub mime_type: String,
    pub accepted: bool,
    pub policy_reason: String,
    #[serde(default)]
    pub data_base64: Option<String>,
    #[serde(default)]
    pub stored_path: Option<String>,
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
    pub reply: String,
    pub attachments: Vec<WebChatAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebChatHistoryMessage {
    pub id: String,
    pub role: String,
    pub title: String,
    pub body: String,
    pub attachments: Vec<WebChatAttachment>,
    pub created_at_ms: u128,
    pub policy_reason: String,
    pub stored_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebChatHistoryResponse {
    pub messages: Vec<WebChatHistoryMessage>,
    pub stored_path: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WebChatLedgerRow {
    message_id: String,
    message: String,
    #[serde(default)]
    attachments: Vec<WebChatAttachment>,
    #[serde(default)]
    assistant_reply: String,
    created_at_ms: u128,
}

const HISTORY_LIMIT: usize = 100;

pub(crate) async fn history_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    match load_chat_history(HISTORY_LIMIT) {
        Ok(history) => (StatusCode::OK, Json(history)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("chat history failed: {err}"),
        )
            .into_response(),
    }
}

pub(crate) async fn submit_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebChatSubmitRequest>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let mut response = evaluate_chat_submit(&request);
    let mut ledger_attachments: Vec<WebChatAttachment> =
        request.attachments.iter().map(metadata_only).collect();
    if response.accepted {
        state.live.record(
            "web.chat.submitted",
            format!("web chat turn {} submitted", response.message_id),
        );
        let Some(backend) = state.chat_backend.clone() else {
            response.accepted = false;
            response.policy_reason = "chat submit denied: web chat runtime unavailable".into();
            return (StatusCode::OK, Json(response)).into_response();
        };
        match backend.submit(&response.message_id, request.clone()).await {
            Ok(output) => {
                response.reply = output.reply;
                response.policy_reason = output.policy_reason;
                ledger_attachments = output.attachments;
                response.attachments = ledger_attachments.clone();
            }
            Err(err) => {
                response.accepted = false;
                response.policy_reason = format!("chat runtime failed: {err}");
                state.live.record(
                    "web.chat.failed",
                    format!("web chat turn {} failed", response.message_id),
                );
            }
        }
    }
    if response.accepted
        && let Err(err) = append_chat_row(&request, &response, &ledger_attachments)
    {
        tracing::warn!("web chat submit append failed: {err}");
    }
    if response.accepted {
        state.live.record(
            "web.chat.completed",
            format!("web chat turn {} completed", response.message_id),
        );
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
    if request
        .attachments
        .iter()
        .any(|attachment| attachment.data_base64.is_none())
    {
        return response(
            false,
            "chat submit denied: attachment bytes were not provided",
            stored_path,
        );
    }

    WebChatSubmitResponse {
        message_id: format!("webmsg_{}", uuid::Uuid::new_v4()),
        accepted: true,
        created_at_ms,
        policy_reason: "chat message accepted by the web session runtime".into(),
        stored_path,
        reply: String::new(),
        attachments: Vec::new(),
    }
}

fn response(accepted: bool, reason: &str, stored_path: String) -> WebChatSubmitResponse {
    WebChatSubmitResponse {
        message_id: String::new(),
        accepted,
        created_at_ms: now_ms(),
        policy_reason: reason.into(),
        stored_path,
        reply: String::new(),
        attachments: Vec::new(),
    }
}

fn append_chat_row(
    request: &WebChatSubmitRequest,
    response: &WebChatSubmitResponse,
    attachments: &[WebChatAttachment],
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
        attachments: attachments.to_vec(),
        assistant_reply: response.reply.clone(),
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

fn load_chat_history(limit: usize) -> anyhow::Result<WebChatHistoryResponse> {
    let Some(path) = chat_ledger_path() else {
        anyhow::bail!("home directory unavailable");
    };
    let stored_path = path.to_string_lossy().to_string();
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WebChatHistoryResponse {
                messages: Vec::new(),
                stored_path,
                truncated: false,
            });
        }
        Err(err) => return Err(err.into()),
    };
    let mut rows = Vec::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        match serde_json::from_str::<WebChatLedgerRow>(line) {
            Ok(row) => rows.push(row),
            Err(err) => tracing::warn!("web chat history skipped malformed row: {err}"),
        }
    }
    let truncated = rows.len() > limit;
    let start = rows.len().saturating_sub(limit);
    Ok(history_from_rows(&rows[start..], stored_path, truncated))
}

fn history_from_rows(
    rows: &[WebChatLedgerRow],
    stored_path: String,
    truncated: bool,
) -> WebChatHistoryResponse {
    let mut messages = Vec::new();
    for row in rows {
        messages.push(WebChatHistoryMessage {
            id: format!("{}:user", row.message_id),
            role: "user".into(),
            title: "You".into(),
            body: if row.message.trim().is_empty() {
                "Attachments".into()
            } else {
                row.message.clone()
            },
            attachments: row.attachments.clone(),
            created_at_ms: row.created_at_ms,
            policy_reason: "stored in web chat ledger".into(),
            stored_path: stored_path.clone(),
        });
        if !row.assistant_reply.trim().is_empty() {
            messages.push(WebChatHistoryMessage {
                id: format!("{}:assistant", row.message_id),
                role: "assistant".into(),
                title: "Archon".into(),
                body: row.assistant_reply.clone(),
                attachments: Vec::new(),
                created_at_ms: row.created_at_ms,
                policy_reason: "restored from web chat ledger".into(),
                stored_path: stored_path.clone(),
            });
        }
    }
    WebChatHistoryResponse {
        messages,
        stored_path,
        truncated,
    }
}

fn metadata_only(attachment: &WebChatAttachment) -> WebChatAttachment {
    WebChatAttachment {
        file_name: attachment.file_name.clone(),
        size_bytes: attachment.size_bytes,
        mime_type: attachment.mime_type.clone(),
        accepted: attachment.accepted,
        policy_reason: attachment.policy_reason.clone(),
        data_base64: None,
        stored_path: attachment.stored_path.clone(),
    }
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
        exported(WebChatHistoryMessage::decl(&cfg)),
        exported(WebChatHistoryResponse::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
