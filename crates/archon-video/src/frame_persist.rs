use archon_docs::models::{ArtifactRecord, ProvenanceEdgeType};
use archon_policy::VideoPolicy;
use cozo::DbInstance;

use crate::chunk_writer::{ARTIFACT_TYPE_VIDEO_FRAME_OCR, ARTIFACT_TYPE_VIDEO_FRAME_VLM};
use crate::dedupe::DedupeGroup;
use crate::errors::VideoError;
use crate::provenance::insert_edge;
use crate::store::{self, FrameDescription, VideoTrack};

pub fn persist_frame_groups(
    db: &DbInstance,
    document_id: &str,
    video_id: &str,
    source_artifact_id: &str,
    groups: &[DedupeGroup],
    policy: &VideoPolicy,
    created_at: &str,
) -> Result<usize, VideoError> {
    if groups.is_empty() || (!policy.frames.ocr && !policy.frames.vlm) {
        return Ok(0);
    }
    let track_id = format!("track-{}", uuid::Uuid::new_v4());
    store::insert_video_track(
        db,
        &VideoTrack {
            track_id: track_id.clone(),
            video_id: video_id.to_string(),
            track_kind: "frames".into(),
            provider: "ffmpeg".into(),
            model: String::new(),
            status: "success".into(),
            warning_count: 0,
            error_count: 0,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        },
    )?;

    for group in groups {
        let artifact_id = insert_frame_artifacts(
            db,
            document_id,
            source_artifact_id,
            policy,
            created_at,
            group,
        )?;
        store::insert_frame_description(
            db,
            &FrameDescription {
                frame_id: format!("frame-{}", uuid::Uuid::new_v4()),
                video_id: video_id.to_string(),
                track_id: track_id.clone(),
                timestamp_ms: group.first_timestamp_ms as i64,
                timestamp_end_ms: group.last_timestamp_ms as i64,
                frame_hash: group.representative.frame_hash.clone(),
                perceptual_hash: format!("{:064b}", group.representative_hash),
                image_artifact_id: artifact_id,
                ocr_text: String::new(),
                vlm_description: String::new(),
                provider: "ffmpeg".into(),
                model: String::new(),
                cost_usd: 0.0,
                chunk_id: String::new(),
                dedupe_group_id: group.dedupe_group_id.clone(),
                status: "pending_ocr_vlm".into(),
                warning: String::new(),
                created_at: created_at.to_string(),
            },
        )?;
    }
    Ok(groups.len())
}

fn insert_frame_artifacts(
    db: &DbInstance,
    document_id: &str,
    source_artifact_id: &str,
    policy: &VideoPolicy,
    created_at: &str,
    group: &DedupeGroup,
) -> Result<String, VideoError> {
    let mut first_artifact_id = String::new();
    for artifact_type in frame_artifact_types(policy) {
        let artifact_id = format!("artifact-{}", uuid::Uuid::new_v4());
        if first_artifact_id.is_empty() {
            first_artifact_id = artifact_id.clone();
        }
        archon_docs::store::insert_artifact(
            db,
            &ArtifactRecord {
                artifact_id: artifact_id.clone(),
                document_id: document_id.to_string(),
                artifact_type: artifact_type.to_string(),
                content_hash: group.representative.frame_hash.clone(),
                created_at: created_at.to_string(),
                provenance_record_id: String::new(),
            },
        )
        .map_err(store_error)?;
        insert_edge(
            db,
            &artifact_id,
            source_artifact_id,
            ProvenanceEdgeType::ExtractedFrom,
        )
        .map_err(store_error)?;
    }
    Ok(first_artifact_id)
}

fn frame_artifact_types(policy: &VideoPolicy) -> Vec<&'static str> {
    let mut types = Vec::new();
    if policy.frames.ocr {
        types.push(ARTIFACT_TYPE_VIDEO_FRAME_OCR);
    }
    if policy.frames.vlm {
        types.push(ARTIFACT_TYPE_VIDEO_FRAME_VLM);
    }
    types
}

fn store_error(error: impl std::fmt::Display) -> VideoError {
    VideoError::Store {
        message: error.to_string(),
    }
}
