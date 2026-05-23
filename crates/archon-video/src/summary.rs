use archon_docs::hash::sha256_str;
use archon_docs::models::{ArtifactRecord, ChunkArtifact};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_policy::EffectivePolicy;
use cozo::DbInstance;

use crate::chunk_writer::ARTIFACT_TYPE_VIDEO_SUMMARY;
use crate::errors::VideoError;
use crate::store::{self, ChunkTimeRef};

pub async fn generate_video_summary(
    provider: &dyn LlmProvider,
    video_id: &str,
    document_id: &str,
    transcript_text: &str,
    visual_evidence: &str,
    duration_ms: i64,
    policy: &EffectivePolicy,
    db: &DbInstance,
) -> Result<Option<String>, VideoError> {
    let decision = policy.video_summary_decision();
    if !decision.allowed {
        return Ok(None);
    }
    let response = match provider
        .complete(summary_request(transcript_text, visual_evidence))
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!("video summary failed: {error}");
            return Ok(None);
        }
    };
    let summary = response_text(&response.content);
    if summary.trim().is_empty() {
        return Ok(None);
    }
    persist_summary(db, video_id, document_id, &summary, duration_ms)?;
    Ok(Some(summary))
}

fn summary_request(transcript_text: &str, visual_evidence: &str) -> LlmRequest {
    LlmRequest {
        messages: vec![serde_json::json!({
            "role": "user",
            "content": format!(
                "Summarize this video for an evidence engine. Include the spoken thesis, \
                 key visual or chart evidence with timestamps when present, and uncertainty. \
                 Do not fabricate beyond the supplied evidence.\n\nTRANSCRIPT:\n{transcript_text}\n\nVISUAL EVIDENCE:\n{visual_evidence}"
            ),
        })],
        ..LlmRequest::default()
    }
}

fn response_text(content: &[serde_json::Value]) -> String {
    content
        .iter()
        .filter_map(|value| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .or_else(|| value.as_str())
        })
        .collect::<Vec<_>>()
        .join("")
}

fn persist_summary(
    db: &DbInstance,
    video_id: &str,
    document_id: &str,
    summary: &str,
    duration_ms: i64,
) -> Result<(), VideoError> {
    let artifact_id = format!("artifact-video-summary-{}", uuid::Uuid::new_v4());
    let created_at = chrono::Utc::now().to_rfc3339();
    archon_docs::store::insert_artifact(
        db,
        &ArtifactRecord {
            artifact_id: artifact_id.clone(),
            document_id: document_id.to_string(),
            artifact_type: ARTIFACT_TYPE_VIDEO_SUMMARY.into(),
            content_hash: sha256_str(summary),
            created_at: created_at.clone(),
            provenance_record_id: String::new(),
        },
    )
    .map_err(store_error)?;
    let chunk_id = format!("chunk-video-summary-{}", uuid::Uuid::new_v4());
    archon_docs::store::insert_chunk(
        db,
        &ChunkArtifact {
            chunk_id: chunk_id.clone(),
            document_id: document_id.to_string(),
            artifact_id,
            chunk_index: 0,
            page_start: 0,
            page_end: 0,
            content: summary.to_string(),
            content_hash: sha256_str(summary),
            embedding_status: "pending".into(),
        },
    )
    .map_err(store_error)?;
    store::insert_chunk_timeref(
        db,
        &ChunkTimeRef {
            chunk_id,
            video_id: video_id.to_string(),
            track_id: "summary".into(),
            timestamp_start_ms: 0,
            timestamp_end_ms: duration_ms,
            created_at,
        },
    )?;
    Ok(())
}

fn store_error(error: impl std::fmt::Display) -> VideoError {
    VideoError::Store {
        message: error.to_string(),
    }
}
