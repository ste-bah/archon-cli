use super::*;

fn stored(ingest: Result<IngestSummary>) -> StoredAttachment {
    StoredAttachment {
        name: "report.pdf".into(),
        mime: "application/pdf".into(),
        bytes: 5,
        path: PathBuf::from("/tmp/report.pdf"),
        text_preview: None,
        ingest,
    }
}

fn summary(document_id: &str) -> IngestSummary {
    IngestSummary {
        document_id: document_id.into(),
        was_new: true,
        warnings: Vec::new(),
        chunks: Vec::new(),
    }
}

#[test]
fn safe_file_name_adds_extension_for_extensionless_upload() {
    assert_eq!(safe_file_name("scan", "application/pdf"), "scan.pdf");
}

#[test]
fn safe_file_name_replaces_path_separators() {
    assert_eq!(
        safe_file_name("../secret.png", "image/png"),
        ".._secret.png"
    );
}

#[test]
fn attachment_metadata_carries_the_ingested_document_id() {
    let metadata = stored(Ok(summary("doc_9f2c"))).metadata();
    assert_eq!(metadata.document_id.as_deref(), Some("doc_9f2c"));
    assert_eq!(metadata.file_name, "report.pdf");
    assert!(metadata.data_base64.is_none());
}

#[test]
fn attachment_metadata_has_no_document_id_when_ingest_failed() {
    let metadata = stored(Err(anyhow::anyhow!("ingest refused by policy"))).metadata();
    assert!(
        metadata.document_id.is_none(),
        "a failed ingest must not report an id"
    );
}

#[test]
fn attachment_metadata_never_reports_an_empty_document_id() {
    let metadata = stored(Ok(summary(""))).metadata();
    assert!(
        metadata.document_id.is_none(),
        "an empty id is absence, not evidence"
    );
}
