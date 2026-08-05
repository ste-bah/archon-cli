use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Document status
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DocumentStatus {
    Discovered,
    Ingesting,
    Ingested,
    Processing,
    Processed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OcrStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MediaKind {
    PageImage,
    EmbeddedImage,
    Figure,
    TableImage,
    Screenshot,
    Chart,
    Diagram,
    ScannedTextRegion,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProvenanceEdgeType {
    DerivedFrom,
    Contains,
    ExtractedFrom,
    Describes,
    Cites,
}

// ---------------------------------------------------------------------------
// Core document types (per TSPEC-ARCHON-EVIDENCE-ENGINE-001 §3)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceDocument {
    pub document_id: String,
    pub source_path: String,
    pub media_type: String,
    pub content_hash: String,
    pub discovered_at: String,
    pub status: DocumentStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OcrRun {
    pub ocr_run_id: String,
    pub document_id: String,
    pub provider: String,
    pub mode: String,
    pub status: OcrStatus,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageOffset {
    pub page: u32,
    pub char_start: usize,
    pub char_end: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OcrResult {
    pub artifact_id: String,
    pub ocr_run_id: String,
    pub document_id: String,
    pub extracted_text: String,
    pub text_length: usize,
    pub page_count: u32,
    pub content_hash: String,
    pub page_offsets: Vec<PageOffset>,
    pub processing_duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PageArtifact {
    pub page_id: String,
    pub document_id: String,
    pub page_number: u32,
    pub text_hash: Option<String>,
    pub image_hash: Option<String>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub provenance_record_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkArtifact {
    pub chunk_id: String,
    pub document_id: String,
    pub artifact_id: String,
    pub chunk_index: u32,
    pub page_start: u32,
    pub page_end: u32,
    pub content: String,
    pub content_hash: String,
    pub embedding_status: String,
}

/// Per-chunk spatial provenance (verbatim-provenance spec §2). Keyed by `chunk_id`,
/// joined at query time — never migrates `doc_chunks`. `super_box`/`blocks` are
/// JSON-encoded (Cozo has no Json column → String, resolution #2). `coord_space`
/// records the origin/scale provenance ("marker" | "pdf_topleft" | "none").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkSpatial {
    pub chunk_id: String,
    pub page_num: u32,
    pub super_box: String,
    pub blocks: String,
    pub coord_space: String,
    pub spatial_hash: String,
}

/// Per-chunk integrity hashes (verbatim-provenance spec §2). Resolution #4: Archon does
/// no text cleaning, so `clean_sha256 == doc_chunks.content_hash` and is referenced there,
/// not duplicated here. `commit_hash` binds text + spatial into the provenance chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkHashes {
    pub chunk_id: String,
    pub raw_sha256: String,
    pub cleaning_version: String,
    pub commit_hash: String,
}

/// Per-chunk layout block (P2): one row per Marker/parser block within a chunk, carrying
/// its BYTE range in `doc_chunks.content` and its bounding box. Feeds sentence-tight bbox
/// derivation and per-block locator offsets. Stored in `doc_chunk_blocks`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkBlock {
    pub chunk_id: String,
    pub block_idx: u32,
    pub char_start: usize,
    pub char_end: usize,
    pub page: u32,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub block_type: String,
    pub text_hash: String,
}

/// A page-boundary transition inside a chunk (P2): logical `page` begins at BYTE
/// `offset_in_chunk` of `doc_chunks.content`. One row per transition (the first row is
/// `offset_in_chunk = 0`). Stored in `doc_chunk_page_breaks`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageBreak {
    pub chunk_id: String,
    pub offset_in_chunk: usize,
    pub page: u32,
}

/// What kind of running-head locator was captured (ingestion-ports spec §4b).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocatorKind {
    PageNumber,
    Bekker,
    /// A side-margin Bekker line-number (the every-5-lines marginalia) — NOT a
    /// page number. Kept distinct so the ~2000 misclassified rows in the audited
    /// corpus can be de-noised at ingest (P2 Bekker/locator fix).
    LineNumber,
}

impl LocatorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PageNumber => "PageNumber",
            Self::Bekker => "Bekker",
            Self::LineNumber => "LineNumber",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "Bekker" => Self::Bekker,
            "LineNumber" => Self::LineNumber,
            _ => Self::PageNumber,
        }
    }
}

/// A citation locator captured from a page's running head (Bekker number e.g. "1147a",
/// or a plain page number) and removed from body text. Anchors verbatim citations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Locator {
    pub locator_id: String,
    pub document_id: String,
    pub page_num: u32,
    pub kind: LocatorKind,
    pub value: String,
    pub bbox: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageDescription {
    pub artifact_id: String,
    pub document_id: String,
    pub page_number: u32,
    pub provider: String,
    pub model: String,
    pub description: String,
    pub created_at: String,
    pub cost_usd: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PdfIngestMetrics {
    pub document_id: String,
    pub embedded_images_extracted: u32,
    pub embedded_images_skipped_filter: u32,
    pub image_ocr_runs: u32,
    pub image_ocr_failures: u32,
    pub image_vlm_descriptions: u32,
    pub image_vlm_failures: u32,
    pub pages_rendered: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaItem {
    pub media_id: String,
    pub document_id: String,
    pub parent_artifact_id: Option<String>,
    pub kind: MediaKind,
    pub page: Option<u32>,
    pub bbox: Option<BoundingBox>,
    pub sha256: String,
    pub mime_type: String,
    pub storage_path: String,
    pub extraction_method: String,
    pub provenance_record_id: String,
}

// ---------------------------------------------------------------------------
// Provenance types (per TSPEC §4)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvenanceRecord {
    pub record_id: String,
    pub artifact_id: String,
    pub artifact_type: String,
    pub operation: String,
    pub input_hashes: Vec<String>,
    pub output_hash: String,
    pub parent_record_ids: Vec<String>,
    pub tool_name: Option<String>,
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub parameters_json: serde_json::Value,
    pub timestamp: String,
    pub chain_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub artifact_id: String,
    pub document_id: String,
    pub artifact_type: String,
    pub content_hash: String,
    pub created_at: String,
    pub provenance_record_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProvenanceEdge {
    pub edge_id: String,
    pub from_artifact_id: String,
    pub to_artifact_id: String,
    pub edge_type: ProvenanceEdgeType,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Processing job
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessingJob {
    pub job_id: String,
    pub document_id: String,
    pub job_type: String,
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
}
