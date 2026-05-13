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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct WebChatLedgerRow {
    message_id: String,
    message: String,
    attachments: Vec<WebChatAttachment>,
    assistant_reply: String,
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
    let mut response = evaluate_chat_submit(&request);
    let mut ledger_attachments: Vec<WebChatAttachment> =
        request.attachments.iter().map(metadata_only).collect();
    if response.accepted {
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
            }
            Err(err) => {
                response.accepted = false;
                response.policy_reason = format!("chat runtime failed: {err}");
            }
        }
    }
    if response.accepted
        && let Err(err) = append_chat_row(&request, &response, &ledger_attachments)
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
                data_base64: None,
                stored_path: None,
            }],
        });
        assert!(!response.accepted);
    }

    #[test]
    fn chat_submit_accepts_attachment_bytes() {
        let response = evaluate_chat_submit(&WebChatSubmitRequest {
            message: String::new(),
            attachments: vec![WebChatAttachment {
                file_name: "notes.md".into(),
                size_bytes: 5,
                mime_type: "text/markdown".into(),
                accepted: true,
                policy_reason: "ok".into(),
                data_base64: Some("aGVsbG8=".into()),
                stored_path: None,
            }],
        });
        assert!(response.accepted);
    }

    #[test]
    fn chat_submit_rejects_attachment_without_bytes() {
        let response = evaluate_chat_submit(&WebChatSubmitRequest {
            message: String::new(),
            attachments: vec![WebChatAttachment {
                file_name: "notes.txt".into(),
                size_bytes: 5,
                mime_type: "text/plain".into(),
                accepted: true,
                policy_reason: "ok".into(),
                data_base64: None,
                stored_path: None,
            }],
        });
        assert!(!response.accepted);
    }
}
