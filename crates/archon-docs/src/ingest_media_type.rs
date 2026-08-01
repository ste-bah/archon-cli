use std::path::Path;

/// Detect media type from file extension.
pub fn detect_media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "tiff" | "tif" => "image/tiff",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "jsonl" | "ndjson" => "application/x-ndjson",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Determine whether a media type is supported for Phase 1 ingest.
pub fn is_supported_media_type(media_type: &str) -> bool {
    is_text_pipeline_media_type(media_type)
        || is_spreadsheet_media_type(media_type)
        || matches!(
            media_type,
            "application/pdf" | "image/png" | "image/jpeg" | "image/tiff"
        )
}

/// Determine whether the media type can go through the OCR → chunk pipeline.
pub(crate) fn is_ocr_runnable(media_type: &str) -> bool {
    is_supported_media_type(media_type)
}

pub(crate) fn is_text_pipeline_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "text/plain"
            | "text/markdown"
            | "text/html"
            | "application/json"
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

pub(crate) fn is_image_media_type(media_type: &str) -> bool {
    matches!(media_type, "image/png" | "image/jpeg" | "image/tiff")
}

/// Spreadsheet media types rendered to Markdown tables before entering the
/// text chunk pipeline (see `ingest_spreadsheet`).
pub(crate) fn is_spreadsheet_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-excel"
            | "text/csv"
            | "text/tab-separated-values"
    )
}
