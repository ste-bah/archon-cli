use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use archon_sdk::web::chat::{
    WebChatAttachment, WebChatBackend, WebChatBackendOutput, WebChatSubmitRequest,
};
use async_trait::async_trait;
use base64::Engine;
use cozo::DbInstance;

use crate::cli_args::Cli;
use crate::session::WebSessionHandle;

const MAX_SNIPPET_CHARS: usize = 12_000;
const MAX_CHUNKS_PER_ATTACHMENT: usize = 6;

pub(crate) struct WebChatBridge {
    session: Arc<WebSessionHandle>,
    cwd: PathBuf,
}

impl WebChatBridge {
    pub(crate) async fn new(
        config: &archon_core::config::ArchonConfig,
        cli: &Cli,
        env_vars: &archon_core::env_vars::ArchonEnvVars,
        resolved_flags: &archon_core::cli_flags::ResolvedFlags,
    ) -> Result<Self> {
        let session_id = format!("web-{}", uuid::Uuid::new_v4());
        let session =
            crate::session::spawn_web_session(config, &session_id, cli, env_vars, resolved_flags)
                .await?;
        Ok(Self {
            session,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        })
    }
}

#[async_trait]
impl WebChatBackend for WebChatBridge {
    async fn submit(
        &self,
        message_id: &str,
        request: WebChatSubmitRequest,
    ) -> Result<WebChatBackendOutput> {
        let (prompt, attachments) = if request.attachments.is_empty() {
            (request.message.trim().to_string(), Vec::new())
        } else {
            self.prompt_with_attachments(message_id, &request).await?
        };
        let reply = self.session.submit(prompt).await?;
        Ok(WebChatBackendOutput {
            reply,
            policy_reason: "chat message handled by the live Archon session".into(),
            attachments,
        })
    }
}

impl WebChatBridge {
    async fn prompt_with_attachments(
        &self,
        message_id: &str,
        request: &WebChatSubmitRequest,
    ) -> Result<(String, Vec<WebChatAttachment>)> {
        let db = open_docs_db()?;
        let policy = archon_policy::load_effective_policy(&self.cwd).unwrap_or_default();
        let _ = archon_docs::embed::init_default_provider();
        let vlm_report = archon_docs::vlm::factory::configure_registered_provider(&policy);

        let mut stored = Vec::new();
        for attachment in &request.attachments {
            stored.push(
                store_and_ingest_attachment(message_id, attachment, &db, &policy)
                    .await
                    .with_context(|| format!("failed to store {}", attachment.file_name))?,
            );
        }

        let mut prompt = String::new();
        let message = request.message.trim();
        if message.is_empty() {
            prompt.push_str("Use the uploaded files as context.");
        } else {
            prompt.push_str(message);
        }
        prompt.push_str("\n\nUploaded files were stored locally and ingested into the Archon docs store. Use them as first-class context for this turn.\n");
        prompt.push_str(&format!(
            "VLM enrichment: {} ({})\n",
            vlm_report.provider, vlm_report.message
        ));

        let ledger_attachments = stored.iter().map(StoredAttachment::metadata).collect();
        for item in stored {
            prompt.push_str(&format!(
                "\n## Attachment: {}\n- MIME: {}\n- Bytes: {}\n- Stored path: {}\n",
                item.name,
                item.mime,
                item.bytes,
                item.path.display()
            ));
            match item.ingest {
                Ok(summary) => {
                    prompt.push_str(&format!(
                        "- Docs document_id: {} ({})\n",
                        summary.document_id,
                        if summary.was_new {
                            "new ingest"
                        } else {
                            "existing duplicate"
                        }
                    ));
                    for warning in summary.warnings {
                        prompt.push_str(&format!("- Ingest warning: {warning}\n"));
                    }
                    append_chunks(&mut prompt, &summary.chunks);
                }
                Err(error) => {
                    prompt.push_str(&format!("- Docs ingest: {error}\n"));
                    if let Some(text) = item.text_preview {
                        prompt.push_str("\nExtracted text preview:\n");
                        prompt.push_str(&text);
                        if !text.ends_with('\n') {
                            prompt.push('\n');
                        }
                    }
                }
            }
        }
        Ok((prompt, ledger_attachments))
    }
}

struct StoredAttachment {
    name: String,
    mime: String,
    bytes: usize,
    path: PathBuf,
    text_preview: Option<String>,
    ingest: Result<IngestSummary>,
}

impl StoredAttachment {
    fn metadata(&self) -> WebChatAttachment {
        WebChatAttachment {
            file_name: self.name.clone(),
            size_bytes: self.bytes as u64,
            mime_type: self.mime.clone(),
            accepted: true,
            policy_reason: "stored and forwarded to live session".into(),
            data_base64: None,
            stored_path: Some(self.path.to_string_lossy().to_string()),
        }
    }
}

struct IngestSummary {
    document_id: String,
    was_new: bool,
    warnings: Vec<String>,
    chunks: Vec<String>,
}

async fn store_and_ingest_attachment(
    message_id: &str,
    attachment: &WebChatAttachment,
    db: &DbInstance,
    policy: &archon_policy::EffectivePolicy,
) -> Result<StoredAttachment> {
    let bytes = decode_attachment(attachment)?;
    let upload_dir = upload_root()?.join(message_id);
    std::fs::create_dir_all(&upload_dir)?;
    let path = upload_dir.join(safe_file_name(&attachment.file_name, &attachment.mime_type));
    std::fs::write(&path, &bytes)?;
    let text_preview = text_preview(&attachment.mime_type, &bytes);
    let ingest = ingest_attachment(db, &path, policy).await;
    Ok(StoredAttachment {
        name: attachment.file_name.clone(),
        mime: attachment.mime_type.clone(),
        bytes: bytes.len(),
        path,
        text_preview,
        ingest,
    })
}

fn decode_attachment(attachment: &WebChatAttachment) -> Result<Vec<u8>> {
    let data = attachment
        .data_base64
        .as_deref()
        .context("attachment payload missing base64 bytes")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .context("attachment payload is not valid base64")?;
    if bytes.len() as u64 != attachment.size_bytes {
        anyhow::bail!(
            "attachment size mismatch for {}: declared {}, decoded {}",
            attachment.file_name,
            attachment.size_bytes,
            bytes.len()
        );
    }
    Ok(bytes)
}

async fn ingest_attachment(
    db: &DbInstance,
    path: &Path,
    policy: &archon_policy::EffectivePolicy,
) -> Result<IngestSummary> {
    let result = archon_docs::ingest::ingest_file_with_policy(db, path, policy).await?;
    let mut chunks = archon_docs::store::list_chunks_for_doc(db, &result.document_id)?;
    chunks.sort_by_key(|chunk| chunk.chunk_index);
    Ok(IngestSummary {
        document_id: result.document_id,
        was_new: result.was_new,
        warnings: result.warnings,
        chunks: chunks
            .into_iter()
            .take(MAX_CHUNKS_PER_ATTACHMENT)
            .map(|chunk| truncate_chars(&chunk.content, MAX_SNIPPET_CHARS))
            .collect(),
    })
}

fn append_chunks(prompt: &mut String, chunks: &[String]) {
    if chunks.is_empty() {
        prompt.push_str("- Extracted docs chunks: none available\n");
        return;
    }
    prompt.push_str("\nExtracted docs chunks:\n");
    for (idx, chunk) in chunks.iter().enumerate() {
        prompt.push_str(&format!("\n### Chunk {}\n{}\n", idx + 1, chunk));
    }
}

fn text_preview(mime_type: &str, bytes: &[u8]) -> Option<String> {
    let is_text = mime_type.starts_with("text/")
        || matches!(
            mime_type,
            "application/json" | "application/x-yaml" | "application/yaml"
        );
    is_text.then(|| truncate_chars(&String::from_utf8_lossy(bytes), MAX_SNIPPET_CHARS))
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut out: String = value.chars().take(limit).collect();
    if value.chars().count() > limit {
        out.push_str("\n[truncated]\n");
    }
    out
}

fn safe_file_name(name: &str, mime_type: &str) -> String {
    let mut safe: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if safe.trim_matches('.').is_empty() {
        safe = "upload".into();
    }
    if Path::new(&safe).extension().is_none()
        && let Some(ext) = extension_for_mime(mime_type)
    {
        safe.push('.');
        safe.push_str(ext);
    }
    safe
}

fn extension_for_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "text/plain" => Some("txt"),
        "text/markdown" => Some("md"),
        "application/pdf" => Some("pdf"),
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/tiff" => Some("tiff"),
        "application/json" => Some("json"),
        "application/x-yaml" | "application/yaml" => Some("yaml"),
        _ => None,
    }
}

fn upload_root() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("home directory unavailable")?
        .join(".archon")
        .join("web")
        .join("uploads"))
}

fn open_docs_db() -> Result<DbInstance> {
    let db = crate::command::store_paths::open_evidence_db("document", &["ARCHON_DOCS_DB_PATH"])?;
    archon_docs::schema::ensure_doc_schema(&db)?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
