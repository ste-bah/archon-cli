//! Local OCR provider — direct text passthrough for native-text PDFs,
//! plain-text files, and image OCR through a local `tesseract` binary.

use async_trait::async_trait;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::provider::{OcrExtractResult, OcrProvider, OcrRequest};
use crate::errors::DocsError;
use crate::models::PageOffset;

/// A local-first OCR provider that extracts native text from PDFs via
/// `pdftotext`, falls back to rendering scanned pages via `pdftoppm`, OCRs
/// images with `tesseract`, and treats plain-text files as single-page docs.
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
    let mut command =
        tokio::process::Command::new(command_path("tesseract", "ARCHON_TESSERACT_BIN"));
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

/// Attempt native extraction first, then render image/scanned pages and OCR
/// each rendered page when native text is empty or unavailable.
async fn extract_pdf_native(path: &Path) -> Result<OcrExtractResult, DocsError> {
    let path_str = path.to_string_lossy().to_string();

    // Try pdftotext (from poppler-utils) first
    let output = tokio::process::Command::new(command_path("pdftotext", "ARCHON_PDFTOTEXT_BIN"))
        .arg("-layout")
        .arg(&path_str)
        .arg("-")
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if text.trim().is_empty() {
                return extract_scanned_pdf(path, None).await;
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
            let native_error = format!("pdftotext failed: {}", stderr.trim());
            extract_scanned_pdf(path, Some(native_error)).await
        }
        Err(e) => Err(DocsError::OcrApi {
            message: format!(
                "pdftotext not found. Install poppler-utils or configure an OCR provider. ({e})"
            ),
            status_code: None,
        }),
    }
}

async fn extract_scanned_pdf(
    path: &Path,
    native_error: Option<String>,
) -> Result<OcrExtractResult, DocsError> {
    let started = Instant::now();
    let render_dir = std::env::temp_dir().join(format!("archon-pdf-ocr-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&render_dir)?;
    let render_result = render_pdf_pages(path, &render_dir).await;
    let page_images = match render_result {
        Ok(page_images) => page_images,
        Err(e) => {
            let _ = fs::remove_dir_all(&render_dir);
            return Err(e);
        }
    };

    if page_images.is_empty() {
        let _ = fs::remove_dir_all(&render_dir);
        return Err(DocsError::OcrApi {
            message: with_native_error("PDF page rendering produced no images", native_error),
            status_code: None,
        });
    }

    let mut pages = Vec::with_capacity(page_images.len());
    for image in &page_images {
        match extract_image_with_tesseract(image, None).await {
            Ok(page) => pages.push(page.full_text),
            Err(e) => {
                let _ = fs::remove_dir_all(&render_dir);
                return Err(e);
            }
        }
    }
    let _ = fs::remove_dir_all(&render_dir);

    let full_text = pages.join("\x0C");
    if full_text.trim().is_empty() {
        return Err(DocsError::OcrApi {
            message: with_native_error("scanned PDF OCR produced no text", native_error),
            status_code: None,
        });
    }
    let offsets = compute_pdf_page_offsets(&full_text);
    Ok(OcrExtractResult {
        page_count: offsets.len() as u32,
        page_offsets: offsets,
        full_text,
        processing_duration_ms: started.elapsed().as_millis() as u64,
    })
}

async fn render_pdf_pages(path: &Path, render_dir: &Path) -> Result<Vec<PathBuf>, DocsError> {
    let prefix = render_dir.join("page");
    let output = tokio::process::Command::new(command_path("pdftoppm", "ARCHON_PDFTOPPM_BIN"))
        .arg("-png")
        .arg(path)
        .arg(&prefix)
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!(
                "pdftoppm not found or failed to start for scanned PDF OCR. \
             Install poppler-utils. ({e})"
            ),
            status_code: None,
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DocsError::OcrApi {
            message: format!("pdftoppm PDF render failed: {}", stderr.trim()),
            status_code: Some(output.status.code().unwrap_or(1) as u16),
        });
    }

    let mut images = fs::read_dir(render_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        })
        .collect::<Vec<_>>();
    images.sort();
    Ok(images)
}

fn with_native_error(message: &str, native_error: Option<String>) -> String {
    match native_error {
        Some(error) if !error.trim().is_empty() => format!("{message}; native extraction: {error}"),
        _ => message.to_string(),
    }
}

fn command_path(default: &str, env_key: &str) -> OsString {
    std::env::var_os(env_key).unwrap_or_else(|| OsString::from(default))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    fn set_ocr_command_env(pdftotext: &Path, pdftoppm: &Path, tesseract: &Path, log: &Path) {
        unsafe {
            std::env::set_var("ARCHON_PDFTOTEXT_BIN", pdftotext);
            std::env::set_var("ARCHON_PDFTOPPM_BIN", pdftoppm);
            std::env::set_var("ARCHON_TESSERACT_BIN", tesseract);
            std::env::set_var("ARCHON_OCR_TEST_LOG", log);
        }
    }

    struct OcrEnvGuard;

    impl Drop for OcrEnvGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("ARCHON_PDFTOTEXT_BIN");
                std::env::remove_var("ARCHON_PDFTOPPM_BIN");
                std::env::remove_var("ARCHON_TESSERACT_BIN");
                std::env::remove_var("ARCHON_OCR_TEST_LOG");
            }
        }
    }

    #[tokio::test]
    async fn test_scanned_pdf_fallback_renders_pages_and_ocr_source_of_truth() {
        let temp = tempfile::tempdir().unwrap();
        let log = temp.path().join("commands.log");
        let pdftotext = temp.path().join("pdftotext");
        let pdftoppm = temp.path().join("pdftoppm");
        let tesseract = temp.path().join("tesseract");
        let pdf = temp.path().join("scan.pdf");
        fs::write(&pdf, b"%PDF fake scanned fixture").unwrap();

        write_executable(
            &pdftotext,
            r#"#!/usr/bin/env bash
echo "pdftotext:$*" >> "$ARCHON_OCR_TEST_LOG"
exit 0
"#,
        );
        write_executable(
            &pdftoppm,
            r#"#!/usr/bin/env bash
echo "pdftoppm:$*" >> "$ARCHON_OCR_TEST_LOG"
prefix="${@: -1}"
printf 'png-one' > "${prefix}-1.png"
printf 'png-two' > "${prefix}-2.png"
"#,
        );
        write_executable(
            &tesseract,
            r#"#!/usr/bin/env bash
echo "tesseract:$*" >> "$ARCHON_OCR_TEST_LOG"
case "$1" in
  *-1.png) echo "alpha page OCR" ;;
  *) echo "beta page OCR" ;;
esac
"#,
        );
        set_ocr_command_env(&pdftotext, &pdftoppm, &tesseract, &log);
        let _env_guard = OcrEnvGuard;

        let result = LocalOcrProvider
            .extract(OcrRequest {
                file_path: pdf.to_string_lossy().to_string(),
                document_id: "doc-scan".into(),
                ocr_run_id: "ocr-scan".into(),
                page_range: None,
                language_hint: None,
            })
            .await
            .unwrap();

        assert_eq!(result.page_count, 2);
        assert!(result.full_text.contains("alpha page OCR"));
        assert!(result.full_text.contains("beta page OCR"));
        assert_eq!(result.page_offsets.len(), 2);

        let command_log = fs::read_to_string(log).unwrap();
        assert!(command_log.contains("pdftotext:-layout"));
        assert!(command_log.contains("pdftoppm:-png"));
        assert_eq!(command_log.matches("tesseract:").count(), 2);
    }
}
