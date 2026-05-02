//! OCR provider trait — per TSPEC §6.1.

use async_trait::async_trait;

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
}

#[async_trait]
pub trait OcrProvider: Send + Sync {
    /// Extract text from a document, returning full text with page offsets.
    async fn extract(&self, request: OcrRequest) -> Result<OcrExtractResult, DocsError>;

    /// Human-readable provider name.
    fn name(&self) -> &'static str;
}
