//! OCR provider trait — per TSPEC §6.1.

use async_trait::async_trait;
use std::sync::{Arc, RwLock};

use crate::errors::DocsError;
use crate::models::PageOffset;

/// Input for an OCR extraction request.
#[derive(Clone, Debug)]
pub struct OcrRequest {
    /// Path to the source file on disk.
    pub file_path: String,
    /// Document ID this OCR run belongs to.
    pub document_id: String,
    /// OCR run ID for tracking.
    pub ocr_run_id: String,
    /// Optional page range (1-based, inclusive). None = all pages.
    pub page_range: Option<(u32, u32)>,
    /// Language hint (e.g. "eng", "chi_sim").
    pub language_hint: Option<String>,
}

/// The result of an OCR extraction, annotated with page offsets.
/// This is the common output contract regardless of provider.
#[derive(Clone, Debug)]
pub struct OcrExtractResult {
    pub full_text: String,
    pub page_count: u32,
    pub page_offsets: Vec<PageOffset>,
    pub processing_duration_ms: u64,
    /// S8: which engine produced this text and how it scored. `None` when the
    /// producing path predates scoring (custom providers, native pdftotext).
    pub quality: Option<OcrQualityMeta>,
}

/// S8 provenance for one OCR result: the engine that won, its quality score,
/// and whether the low-quality arbiter escalated to a second engine.
#[derive(Clone, Debug)]
pub struct OcrQualityMeta {
    pub engine: String,
    pub score: f32,
    pub escalated: bool,
    pub note: Option<String>,
}

#[async_trait]
pub trait OcrProvider: Send + Sync {
    /// Extract text from a document, returning full text with page offsets.
    async fn extract(&self, request: OcrRequest) -> Result<OcrExtractResult, DocsError>;

    /// Human-readable provider name.
    fn name(&self) -> &'static str;
}

/// Wall-clock bound on one local OCR-path subprocess (tesseract / RapidOCR / poppler). A wedged
/// external binary must error out (and be killed via `kill_on_drop`), not hang an ingest worker
/// forever. 120s is generous for a single image/page; override with `ARCHON_OCR_TIMEOUT_SECS`.
pub(crate) fn ocr_timeout() -> std::time::Duration {
    let secs = std::env::var("ARCHON_OCR_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);
    std::time::Duration::from_secs(secs)
}

/// Wall-clock bound on a WHOLE-DOCUMENT `pdftoppm` render (one invocation rasterizes every page,
/// no page-range). This is NOT a per-page op, so it must NOT use [`ocr_timeout`]: a 300–600 page
/// scan renders at ~0.5–2s/page and would false-timeout at 120s (total OCR data loss). 1800s
/// still bounds a genuinely wedged render while covering large scans; override with
/// `ARCHON_PDF_RENDER_TIMEOUT_SECS`.
pub(crate) fn pdf_render_timeout() -> std::time::Duration {
    let secs = std::env::var("ARCHON_PDF_RENDER_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1800);
    std::time::Duration::from_secs(secs)
}

static PROVIDER: RwLock<Option<Arc<dyn OcrProvider>>> = RwLock::new(None);

/// Get the currently configured OCR provider, if one has been installed.
pub fn get_provider() -> Option<Arc<dyn OcrProvider>> {
    PROVIDER.read().ok().and_then(|guard| guard.clone())
}

/// Replace the active OCR provider. Primarily used by tests and local adapters.
pub fn set_provider(provider: Box<dyn OcrProvider>) {
    if let Ok(mut guard) = PROVIDER.write() {
        *guard = Some(Arc::from(provider));
    }
}

/// Remove the active OCR provider, falling back to `LocalOcrProvider`.
#[cfg(test)]
pub fn clear_provider() {
    if let Ok(mut guard) = PROVIDER.write() {
        *guard = None;
    }
}
