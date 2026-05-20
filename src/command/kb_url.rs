use anyhow::Result;
use cozo::DbInstance;

pub(crate) async fn ingest_url(db: &DbInstance, source: &str) -> Result<()> {
    let response = reqwest::get(source).await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("URL ingest failed for {source}: HTTP {status}");
    }
    let media_type = resolve_url_media_type(source, response.headers());
    let bytes = response.bytes().await?;

    if archon_docs::ingest::is_supported_media_type(&media_type) {
        let policy = std::env::current_dir()
            .ok()
            .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
            .unwrap_or_default();
        let result = archon_docs::ingest_bytes::ingest_bytes_source_with_policy(
            db,
            source,
            &media_type,
            &bytes,
            &policy,
        )
        .await?;
        print_file_result(db, &result)?;
        return Ok(());
    }

    if is_text_url_media_type(&media_type) {
        let body = String::from_utf8_lossy(&bytes);
        let result = archon_docs::ingest_text::ingest_text_source(db, source, &media_type, &body)?;
        println!("Ingested: {}", result.document_id);
        if !result.was_new {
            println!("Skipped duplicate: true");
        }
        println!("Chunks: {}", result.chunks_registered);
        return Ok(());
    }

    anyhow::bail!(
        "KB URL ingest does not support media type `{media_type}` from {source}. \
         Supported URL media includes text, Markdown, PDF, PNG, JPEG, and TIFF."
    );
}

fn print_file_result(
    db: &DbInstance,
    result: &archon_docs::ingest::IngestFileResult,
) -> Result<()> {
    let chunks = archon_docs::store::list_chunks_for_doc(db, &result.document_id)?;
    if result.was_new {
        println!("Ingested: {}", result.document_id);
    } else {
        println!("Skipped duplicate: true");
        println!("Ingested: {}", result.document_id);
    }
    println!("Chunks: {}", chunks.len());
    if result.pipeline_failed {
        println!("Warning: processing failed; document status is Failed");
    }
    for warning in &result.warnings {
        println!("Warning: {warning}");
    }
    Ok(())
}

fn resolve_url_media_type(source: &str, headers: &reqwest::header::HeaderMap) -> String {
    let header_type = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(normalize_media_type)
        .filter(|media_type| !media_type.is_empty());

    match header_type.as_deref() {
        Some("application/octet-stream") | None => infer_media_type_from_url(source)
            .unwrap_or_else(|| header_type.unwrap_or_else(|| "text/plain".to_string())),
        Some(_) => header_type.unwrap_or_else(|| "text/plain".to_string()),
    }
}

fn normalize_media_type(media_type: &str) -> String {
    media_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn infer_media_type_from_url(source: &str) -> Option<String> {
    let url = reqwest::Url::parse(source).ok()?;
    let ext = url
        .path_segments()?
        .next_back()?
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())?;
    match ext.as_str() {
        "txt" => Some("text/plain".to_string()),
        "md" | "markdown" => Some("text/markdown".to_string()),
        "pdf" => Some("application/pdf".to_string()),
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "tif" | "tiff" => Some("image/tiff".to_string()),
        _ => None,
    }
}

fn is_text_url_media_type(media_type: &str) -> bool {
    let normalized = normalize_media_type(media_type);
    normalized.starts_with("text/")
        || normalized.ends_with("+json")
        || normalized.ends_with("+xml")
        || matches!(
            normalized.as_str(),
            "application/json"
                | "application/ld+json"
                | "application/x-ndjson"
                | "application/xml"
                | "application/xhtml+xml"
                | "application/rss+xml"
                | "application/atom+xml"
                | "application/yaml"
                | "application/x-yaml"
                | "application/toml"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

    #[test]
    fn url_media_type_gate_accepts_text_like_content() {
        for media_type in [
            "text/plain",
            "text/html; charset=utf-8",
            "application/json",
            "application/activity+json",
            "application/rss+xml",
            "application/x-yaml",
        ] {
            assert!(
                is_text_url_media_type(media_type),
                "{media_type} should be accepted"
            );
        }
    }

    #[test]
    fn url_media_type_gate_rejects_unsupported_binary_content() {
        for media_type in ["audio/mpeg", "video/mp4"] {
            assert!(
                !is_text_url_media_type(media_type),
                "{media_type} should be rejected"
            );
        }
    }

    #[test]
    fn url_media_type_resolver_uses_header_when_specific() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/pdf"));
        assert_eq!(
            resolve_url_media_type("https://example.test/file.txt", &headers),
            "application/pdf"
        );
    }

    #[test]
    fn url_media_type_resolver_infers_from_url_for_octet_stream() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        assert_eq!(
            resolve_url_media_type("https://example.test/file.pdf?download=1", &headers),
            "application/pdf"
        );
    }
}
