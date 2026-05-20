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

    anyhow::bail!(
        "KB URL ingest does not support media type `{media_type}` from {source}. \
         Supported URL media includes text, Markdown, HTML, JSON, XML, YAML, TOML, PDF, PNG, JPEG, and TIFF."
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
        "html" | "htm" => Some("text/html".to_string()),
        "json" => Some("application/json".to_string()),
        "jsonl" | "ndjson" => Some("application/x-ndjson".to_string()),
        "xml" => Some("application/xml".to_string()),
        "yaml" | "yml" => Some("application/yaml".to_string()),
        "toml" => Some("application/toml".to_string()),
        "pdf" => Some("application/pdf".to_string()),
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "tif" | "tiff" => Some("image/tiff".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

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

    #[test]
    fn url_media_type_resolver_infers_structured_text_from_url() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        );
        assert_eq!(
            resolve_url_media_type("https://example.test/data.json", &headers),
            "application/json"
        );
        assert_eq!(
            resolve_url_media_type("https://example.test/page.html", &headers),
            "text/html"
        );
    }
}
