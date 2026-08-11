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
            document_id: None,
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
            document_id: None,
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
            document_id: None,
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
                document_id: Some("doc_notes".into()),
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
    assert_eq!(
        response.messages[0].attachments[0].document_id.as_deref(),
        Some("doc_notes")
    );
    assert_eq!(response.messages[1].body, "hi there");
}

#[test]
fn history_rows_hide_legacy_tool_output_noise() {
    let response = history_from_rows(
        &[WebChatLedgerRow {
            message_id: "webmsg_1".into(),
            message: "hello".into(),
            attachments: Vec::new(),
            assistant_reply: "\n[tool] DocSearch started\n\
                [tool] memory_recall done: 10 memories found\n\
                noisy memory row\n\
                \n\
                [tool] DocSearch failed: Error: database is locked\n\
                The document store is locked right now.\n"
                .into(),
            created_at_ms: 1770000000,
        }],
        "/tmp/chat.messages.jsonl".into(),
        false,
    );
    assert_eq!(response.messages.len(), 2);
    assert_eq!(
        response.messages[1].body,
        "The document store is locked right now."
    );
}

fn ingested_attachment(document_id: Option<&str>) -> WebChatAttachment {
    WebChatAttachment {
        file_name: "report.pdf".into(),
        size_bytes: 5,
        mime_type: "application/pdf".into(),
        accepted: true,
        policy_reason: "stored and forwarded to live session".into(),
        data_base64: None,
        stored_path: Some("/tmp/report.pdf".into()),
        document_id: document_id.map(str::to_string),
    }
}

#[test]
fn backend_document_id_reaches_the_submit_response_payload() {
    let request = WebChatSubmitRequest {
        message: "summarise this".into(),
        attachments: vec![WebChatAttachment {
            data_base64: Some("aGVsbG8=".into()),
            ..ingested_attachment(None)
        }],
    };
    let mut response = evaluate_chat_submit(&request);
    assert!(response.accepted);
    assert!(response.attachments.is_empty());

    let ledger = apply_backend_output(
        &mut response,
        WebChatBackendOutput {
            reply: "done".into(),
            policy_reason: "chat message handled by the live Archon session".into(),
            attachments: vec![ingested_attachment(Some("doc_9f2c"))],
        },
    );

    assert_eq!(
        response.attachments[0].document_id.as_deref(),
        Some("doc_9f2c")
    );
    assert_eq!(ledger[0].document_id.as_deref(), Some("doc_9f2c"));

    let wire = serde_json::to_string(&response).expect("serialize submit response");
    assert!(
        wire.contains("\"documentId\":\"doc_9f2c\""),
        "submit response payload lost the document id: {wire}"
    );
}

#[test]
fn attachment_without_ingest_serialises_null_not_an_empty_document_id() {
    let wire = serde_json::to_string(&ingested_attachment(None)).expect("serialize attachment");
    assert!(wire.contains("\"documentId\":null"), "{wire}");
    assert!(!wire.contains("\"documentId\":\"\""), "{wire}");
}

#[test]
fn metadata_only_keeps_the_document_id_and_drops_the_bytes() {
    let stripped = metadata_only(&WebChatAttachment {
        data_base64: Some("aGVsbG8=".into()),
        ..ingested_attachment(Some("doc_9f2c"))
    });
    assert!(stripped.data_base64.is_none());
    assert_eq!(stripped.document_id.as_deref(), Some("doc_9f2c"));
}

#[test]
fn ledger_rows_written_before_the_id_existed_still_load() {
    let row: WebChatLedgerRow = serde_json::from_str(
        r#"{"messageId":"webmsg_1","message":"hi","attachments":[{"fileName":"notes.md",
           "sizeBytes":5,"mimeType":"text/markdown","accepted":true,"policyReason":"ok"}],
           "assistantReply":"","createdAtMs":1770000000}"#,
    )
    .expect("ledger row without a document id parses");
    assert!(row.attachments[0].document_id.is_none());
}
