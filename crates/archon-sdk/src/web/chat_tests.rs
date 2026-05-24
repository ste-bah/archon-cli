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
    assert!(response.attachments.is_empty());
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

#[test]
fn history_rows_restore_user_and_assistant_messages() {
    let response = history_from_rows(
        &[WebChatLedgerRow {
            message_id: "webmsg_1".into(),
            message: "hello".into(),
            attachments: vec![WebChatAttachment {
                file_name: "notes.md".into(),
                size_bytes: 5,
                mime_type: "text/markdown".into(),
                accepted: true,
                policy_reason: "ok".into(),
                data_base64: None,
                stored_path: Some("/tmp/notes.md".into()),
            }],
            assistant_reply: "hi there".into(),
            created_at_ms: 1770000000,
        }],
        "/tmp/chat.messages.jsonl".into(),
        false,
    );
    assert_eq!(response.messages.len(), 2);
    assert_eq!(response.messages[0].role, "user");
    assert_eq!(response.messages[0].attachments.len(), 1);
    assert_eq!(response.messages[1].body, "hi there");
}
