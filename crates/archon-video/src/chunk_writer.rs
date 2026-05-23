use anyhow::Result;
use archon_docs::hash::sha256_str;
use archon_docs::models::{ArtifactRecord, ChunkArtifact};
use cozo::DbInstance;

use crate::store::{self, ChunkTimeRef};
use crate::transcript::TranscriptSegment;

pub const ARTIFACT_TYPE_VIDEO_TRANSCRIPT: &str = "video_transcript";
pub const ARTIFACT_TYPE_VIDEO_FRAME_OCR: &str = "video_frame_ocr";
pub const ARTIFACT_TYPE_VIDEO_FRAME_VLM: &str = "video_frame_vlm";
pub const ARTIFACT_TYPE_VIDEO_SUMMARY: &str = "video_summary";

pub fn write_transcript_artifact(
    db: &DbInstance,
    document_id: &str,
    transcript_text: &str,
    created_at: &str,
) -> Result<String> {
    let artifact_id = format!("artifact-video-transcript-{}", uuid::Uuid::new_v4());
    archon_docs::store::insert_artifact(
        db,
        &ArtifactRecord {
            artifact_id: artifact_id.clone(),
            document_id: document_id.to_string(),
            artifact_type: ARTIFACT_TYPE_VIDEO_TRANSCRIPT.to_string(),
            content_hash: sha256_str(transcript_text),
            created_at: created_at.to_string(),
            provenance_record_id: String::new(),
        },
    )?;
    Ok(artifact_id)
}

pub fn write_transcript_chunk(db: &DbInstance, input: &TranscriptChunkInput<'_>) -> Result<String> {
    let chunk_id = format!("chunk-video-{}-{}", input.video_id, input.chunk_index);
    let chunk = ChunkArtifact {
        chunk_id: chunk_id.clone(),
        document_id: input.document_id.to_string(),
        artifact_id: input.artifact_id.to_string(),
        chunk_index: input.chunk_index,
        page_start: 0,
        page_end: 0,
        content: input.segment.text.clone(),
        content_hash: sha256_str(&input.segment.text),
        embedding_status: "pending".into(),
    };
    archon_docs::store::insert_chunk(db, &chunk)?;
    store::insert_chunk_timeref(
        db,
        &ChunkTimeRef {
            chunk_id: chunk_id.clone(),
            video_id: input.video_id.to_string(),
            track_id: input.track_id.to_string(),
            timestamp_start_ms: input.segment.start_ms as i64,
            timestamp_end_ms: input.segment.end_ms as i64,
            created_at: input.created_at.to_string(),
        },
    )?;
    Ok(chunk_id)
}

pub struct TranscriptChunkInput<'a> {
    pub document_id: &'a str,
    pub artifact_id: &'a str,
    pub video_id: &'a str,
    pub track_id: &'a str,
    pub chunk_index: u32,
    pub segment: &'a TranscriptSegment,
    pub created_at: &'a str,
}
