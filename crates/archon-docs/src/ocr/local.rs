//! Local OCR provider — direct text passthrough for native-text PDFs,
//! plain-text files, and image OCR through a local `tesseract` binary.

use async_trait::async_trait;
use std::fs;
use std::path::Path;
use std::time::Instant;

use super::provider::{OcrExtractResult, OcrProvider, OcrRequest};
use crate::errors::DocsError;
use crate::models::PageOffset;

/// A local-first OCR provider that extracts native text from PDFs
/// (via `pdftotext`) and treats plain-text files as single-page documents.
/// Image-based or scanned PDFs return `DocsError::OcrApi`.
pub struct LocalOcrProvider;

#[async_trait]
impl OcrProvider for LocalOcrProvider {
    fn name(&self) -> &'static str {
        "local"
    }

    async fn extract(&self, request: OcrRequest) -> Result<OcrExtractResult, DocsError> {
        let path = Path::new(&request.file_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        match ext.as_str() {
            "txt" | "md" | "markdown" => extract_text_file(path),
            "pdf" => extract_pdf_native(path).await,
            "png" | "jpg" | "jpeg" | "tif" | "tiff" => {
                extract_image_with_tesseract(path, request.language_hint.as_deref()).await
            }
            _ => Err(DocsError::UnsupportedMediaType { media_type: ext }),
        }
    }
}

async fn extract_image_with_tesseract(
    path: &Path,
    language_hint: Option<&str>,
) -> Result<OcrExtractResult, DocsError> {
    let started = Instant::now();
    let mut command = tokio::process::Command::new("tesseract");
    command.arg(path).arg("stdout");
    if let Some(language) = language_hint.filter(|s| !s.trim().is_empty()) {
        command.arg("-l").arg(language);
    }

    let output = command.output().await.map_err(|e| DocsError::OcrApi {
        message: format!(
            "tesseract not found or failed to start for image OCR. Install tesseract-ocr. ({e})"
        ),
        status_code: None,
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DocsError::OcrApi {
            message: format!("tesseract image OCR failed: {}", stderr.trim()),
            status_code: Some(output.status.code().unwrap_or(1) as u16),
        });
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        return Err(DocsError::OcrApi {
            message: "tesseract image OCR produced no text".into(),
            status_code: None,
        });
    }

    Ok(OcrExtractResult {
        page_count: 1,
        page_offsets: vec![PageOffset {
            page: 1,
            char_start: 0,
            char_end: text.len(),
        }],
        full_text: text,
        processing_duration_ms: started.elapsed().as_millis() as u64,
    })
}

fn extract_text_file(path: &Path) -> Result<OcrExtractResult, DocsError> {
    let content = fs::read_to_string(path)?;
    let offsets = compute_pdf_page_offsets(&content);
    let page_count = offsets.len() as u32;
    Ok(OcrExtractResult {
        page_count,
        page_offsets: offsets,
        full_text: content,
        processing_duration_ms: 0,
    })
}

/// Attempt to extract text from a native-text PDF via `pdftotext`.
/// Returns `DocsError::OcrApi` if the tool is not installed or the PDF
/// has no extractable text (scanned/image PDFs).
async fn extract_pdf_native(path: &Path) -> Result<OcrExtractResult, DocsError> {
    let path_str = path.to_string_lossy().to_string();

    // Try pdftotext (from poppler-utils) first
    let output = tokio::process::Command::new("pdftotext")
        .arg("-layout")
        .arg(&path_str)
        .arg("-")
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if text.trim().is_empty() {
                return Err(DocsError::OcrApi {
                    message: "PDF produced no extractable text — may be image-only. \
                         Install ocrmypdf or tesseract for OCR."
                        .into(),
                    status_code: None,
                });
            }
            let _text_len = text.len();
            // Parse pdftotext -layout output to find page boundaries (form feed chars)
            let offsets = compute_pdf_page_offsets(&text);
            let page_count = offsets.len() as u32;
            Ok(OcrExtractResult {
                full_text: text,
                page_count,
                page_offsets: offsets,
                processing_duration_ms: 0,
            })
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(DocsError::OcrApi {
                message: format!("pdftotext failed: {}", stderr.trim()),
                status_code: Some(out.status.code().unwrap_or(1) as u16),
            })
        }
        Err(e) => Err(DocsError::OcrApi {
            message: format!(
                "pdftotext not found. Install poppler-utils or configure an OCR provider. ({e})"
            ),
            status_code: None,
        }),
    }
}

/// Split text at form-feed characters (\x0C) to determine page boundaries.
fn compute_pdf_page_offsets(text: &str) -> Vec<PageOffset> {
    let mut offsets = Vec::new();
    let mut page = 1u32;
    let mut char_start = 0usize;

    for (i, ch) in text.char_indices() {
        if ch == '\x0C' {
            offsets.push(PageOffset {
                page,
                char_start,
                char_end: i,
            });
            page += 1;
            char_start = i + 1;
        }
    }

    // Final page (or entire document if no form feeds)
    let text_len = text.len();
    if char_start < text_len || offsets.is_empty() {
        offsets.push(PageOffset {
            page,
            char_start,
            char_end: text_len,
        });
    }

    offsets
}
