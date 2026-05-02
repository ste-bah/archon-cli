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
