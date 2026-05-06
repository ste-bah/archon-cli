use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use archon_policy::PdfPolicy;
use tokio::process::Command;

use crate::errors::DocsError;
use crate::hash::sha256_hex;
use crate::models::PageOffset;

#[derive(Clone, Debug)]
pub struct PdfExtractResult {
    pub full_text: String,
    pub page_count: u32,
    pub page_offsets: Vec<PageOffset>,
    pub embedded_images: Vec<PdfImage>,
    pub rendered_pages: Vec<PdfImage>,
    pub embedded_images_skipped_filter: usize,
    pub warnings: Vec<String>,
    pub processing_duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfImage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub source_page: u32,
    pub source_pages: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub origin: PdfImageOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfImageOrigin {
    Embedded { xobject_name: Option<String> },
    RenderedPage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfImagesListEntry {
    pub source_page: u32,
    pub source_pages: Vec<u32>,
    pub width: u32,
    pub height: u32,
    pub object_key: Option<String>,
    pub xobject_name: Option<String>,
}

pub async fn extract_pdf_unified(
    path: &Path,
    pdf_policy: &PdfPolicy,
) -> Result<PdfExtractResult, DocsError> {
    let started = Instant::now();
    let mut warnings = Vec::new();
    let text_result = extract_text_layer(path).await;
    let text_error = text_result.as_ref().err().map(ToString::to_string);
    let full_text = match text_result {
        Ok(text) => text,
        Err(e) => {
            warnings.push(format!("pdftotext skipped: {e}"));
            String::new()
        }
    };

    let mut embedded_images = Vec::new();
    let mut embedded_images_skipped_filter = 0usize;
    if pdf_policy.extract_embedded_images {
        match extract_embedded_images(path, pdf_policy).await {
            Ok(result) => {
                embedded_images = result.images;
                embedded_images_skipped_filter = result.skipped_filter;
                warnings.extend(result.warnings);
            }
            Err(e) => warnings.push(format!("embedded image extraction skipped: {e}")),
        }
    }

    let should_render_pages = pdf_policy.render_text_pdf_pages
        || (full_text.trim().is_empty() && embedded_images.is_empty());
    let mut rendered_pages = Vec::new();
    if should_render_pages {
        match render_pdf_pages(path).await {
            Ok(images) => rendered_pages = images,
            Err(e) if full_text.trim().is_empty() && embedded_images.is_empty() => {
                return Err(match text_error {
                    Some(text_error) => DocsError::OcrApi {
                        message: format!("{text_error}; PDF render fallback also failed: {e}"),
                        status_code: None,
                    },
                    None => e,
                });
            }
            Err(e) => warnings.push(format!("PDF render fallback skipped: {e}")),
        }
    }

    let mut page_offsets = if full_text.trim().is_empty() {
        Vec::new()
    } else {
        compute_pdf_page_offsets(&full_text)
    };
    let max_image_page = embedded_images
        .iter()
        .chain(rendered_pages.iter())
        .flat_map(|image| image.source_pages.iter().copied())
        .max()
        .unwrap_or(0);
    let page_count = page_offsets.len().max(max_image_page as usize).max(1) as u32;
    if page_offsets.is_empty() {
        page_offsets = empty_page_offsets(page_count);
    } else {
        let existing = page_offsets.len() as u32;
        for page in (existing + 1)..=page_count {
            page_offsets.push(PageOffset {
                page,
                char_start: 0,
                char_end: 0,
            });
        }
    }

    Ok(PdfExtractResult {
        full_text,
        page_count,
        page_offsets,
        embedded_images,
        rendered_pages,
        embedded_images_skipped_filter,
        warnings,
        processing_duration_ms: started.elapsed().as_millis() as u64,
    })
}

async fn extract_text_layer(path: &Path) -> Result<String, DocsError> {
    let output = Command::new(command_path("pdftotext", "ARCHON_PDFTOTEXT_BIN"))
        .arg("-layout")
        .arg(path)
        .arg("-")
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("pdftotext not found. Install poppler-utils. ({e})"),
            status_code: None,
        })?;
    if !output.status.success() {
        return Err(DocsError::OcrApi {
            message: format!(
                "pdftotext failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            status_code: output.status.code().map(|code| code as u16),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

struct EmbeddedImageExtraction {
    images: Vec<PdfImage>,
    skipped_filter: usize,
    warnings: Vec<String>,
}

async fn extract_embedded_images(
    path: &Path,
    pdf_policy: &PdfPolicy,
) -> Result<EmbeddedImageExtraction, DocsError> {
    let list_output = Command::new(command_path("pdfimages", "ARCHON_PDFIMAGES_BIN"))
        .arg("-list")
        .arg(path)
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("pdfimages not found. Install poppler-utils. ({e})"),
            status_code: None,
        })?;
    if !list_output.status.success() {
        return Err(DocsError::OcrApi {
            message: format!(
                "pdfimages -list failed: {}",
                String::from_utf8_lossy(&list_output.stderr).trim()
            ),
            status_code: list_output.status.code().map(|code| code as u16),
        });
    }
    let entries = parse_pdfimages_list(&String::from_utf8_lossy(&list_output.stdout));
    if entries.is_empty() {
        return Ok(EmbeddedImageExtraction {
            images: Vec::new(),
            skipped_filter: 0,
            warnings: Vec::new(),
        });
    }

    let extract_dir =
        std::env::temp_dir().join(format!("archon-pdf-images-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&extract_dir)?;
    let prefix = extract_dir.join("img");
    let extract_output = Command::new(command_path("pdfimages", "ARCHON_PDFIMAGES_BIN"))
        .arg("-png")
        .arg(path)
        .arg(&prefix)
        .output()
        .await;
    let extract_output = match extract_output {
        Ok(output) => output,
        Err(e) => {
            let _ = fs::remove_dir_all(&extract_dir);
            return Err(DocsError::OcrApi {
                message: format!("pdfimages failed to start extraction: {e}"),
                status_code: None,
            });
        }
    };
    if !extract_output.status.success() {
        let stderr = String::from_utf8_lossy(&extract_output.stderr);
        let _ = fs::remove_dir_all(&extract_dir);
        return Err(DocsError::OcrApi {
            message: format!("pdfimages -png failed: {}", stderr.trim()),
            status_code: extract_output.status.code().map(|code| code as u16),
        });
    }

    let files = list_supported_image_files(&extract_dir)?;
    let mut warnings = Vec::new();
    if files.len() < entries.len() {
        warnings.push(format!(
            "pdfimages listed {} image(s) but extracted {} supported PNG/JPEG file(s)",
            entries.len(),
            files.len()
        ));
    }
    let aligned_entries = if files.len() == entries.len() {
        entries
    } else {
        dedupe_entries_by_object(entries)
    };

    let mut images_by_hash: BTreeMap<String, PdfImage> = BTreeMap::new();
    let mut skipped_filter = 0usize;
    for (entry, file) in aligned_entries.iter().zip(files.iter()) {
        let bytes = fs::read(file)?;
        if !image_survives_filter(entry.width, entry.height, bytes.len() as u64, pdf_policy) {
            skipped_filter += 1;
            continue;
        }
        let hash = sha256_hex(&bytes);
        if let Some(existing) = images_by_hash.get_mut(&hash) {
            for page in &entry.source_pages {
                if !existing.source_pages.contains(page) {
                    existing.source_pages.push(*page);
                }
            }
            continue;
        }
        images_by_hash.insert(
            hash,
            PdfImage {
                bytes,
                mime: mime_from_path(file).unwrap_or("image/png"),
                source_page: entry.source_page,
                source_pages: entry.source_pages.clone(),
                width: entry.width,
                height: entry.height,
                origin: PdfImageOrigin::Embedded {
                    xobject_name: entry.xobject_name.clone(),
                },
            },
        );
    }
    let _ = fs::remove_dir_all(&extract_dir);
    Ok(EmbeddedImageExtraction {
        images: images_by_hash.into_values().collect(),
        skipped_filter,
        warnings,
    })
}

async fn render_pdf_pages(path: &Path) -> Result<Vec<PdfImage>, DocsError> {
    let render_dir =
        std::env::temp_dir().join(format!("archon-pdf-render-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&render_dir)?;
    let prefix = render_dir.join("page");
    let output = Command::new(command_path("pdftoppm", "ARCHON_PDFTOPPM_BIN"))
        .arg("-png")
        .arg(path)
        .arg(&prefix)
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("pdftoppm not found or failed to start for PDF render. ({e})"),
            status_code: None,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_dir_all(&render_dir);
        return Err(DocsError::OcrApi {
            message: format!("pdftoppm PDF render failed: {}", stderr.trim()),
            status_code: output.status.code().map(|code| code as u16),
        });
    }
    let files = list_supported_image_files(&render_dir)?;
    let mut images = Vec::with_capacity(files.len());
    for (index, file) in files.iter().enumerate() {
        let bytes = fs::read(file)?;
        let (width, height) = image_dimensions(&bytes).unwrap_or((0, 0));
        let page = index as u32 + 1;
        images.push(PdfImage {
            bytes,
            mime: mime_from_path(file).unwrap_or("image/png"),
            source_page: page,
            source_pages: vec![page],
            width,
            height,
            origin: PdfImageOrigin::RenderedPage,
        });
    }
    let _ = fs::remove_dir_all(&render_dir);
    Ok(images)
}

pub fn parse_pdfimages_list(output: &str) -> Vec<PdfImagesListEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let cols = line.split_whitespace().collect::<Vec<_>>();
        if cols.len() < 5 {
            continue;
        }
        let Ok(page) = cols[0].parse::<u32>() else {
            continue;
        };
        let Ok(width) = cols[3].parse::<u32>() else {
            continue;
        };
        let Ok(height) = cols[4].parse::<u32>() else {
            continue;
        };
        let object_key = if cols.len() > 11 {
            Some(format!("{}:{}", cols[10], cols[11]))
        } else {
            None
        };
        entries.push(PdfImagesListEntry {
            source_page: page,
            source_pages: vec![page],
            width,
            height,
            object_key: object_key.clone(),
            xobject_name: object_key,
        });
    }
    entries
}

fn dedupe_entries_by_object(entries: Vec<PdfImagesListEntry>) -> Vec<PdfImagesListEntry> {
    let mut deduped = Vec::<PdfImagesListEntry>::new();
    for entry in entries {
        let key = entry.object_key.clone();
        if let Some(key) = key
            && let Some(existing) = deduped
                .iter_mut()
                .find(|candidate| candidate.object_key.as_ref() == Some(&key))
        {
            for page in entry.source_pages {
                if !existing.source_pages.contains(&page) {
                    existing.source_pages.push(page);
                }
            }
            continue;
        }
        deduped.push(entry);
    }
    deduped
}

pub fn image_survives_filter(width: u32, height: u32, bytes: u64, policy: &PdfPolicy) -> bool {
    width.max(height) >= policy.min_image_dimension && bytes >= policy.min_image_bytes
}

fn list_supported_image_files(dir: &Path) -> Result<Vec<PathBuf>, DocsError> {
    let mut files = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| mime_from_path(path).is_some())
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn mime_from_path(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("png") => Some("image/png"),
        Some(ext) if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") => {
            Some("image/jpeg")
        }
        _ => None,
    }
}

fn image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((width, height));
    }
    None
}

fn command_path(default: &str, env_key: &str) -> OsString {
    std::env::var_os(env_key).unwrap_or_else(|| OsString::from(default))
}

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

fn empty_page_offsets(page_count: u32) -> Vec<PageOffset> {
    (1..=page_count)
        .map(|page| PageOffset {
            page,
            char_start: 0,
            char_end: 0,
        })
        .collect()
}

#[cfg(test)]
#[path = "pdf_tests.rs"]
mod pdf_tests;
