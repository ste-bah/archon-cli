use std::path::{Path, PathBuf};

use archon_docs::hash::sha256_str;
use archon_docs::models::{ChunkArtifact, ProvenanceEdgeType};
use archon_docs::ocr::provider::{OcrProvider, OcrRequest};
use archon_docs::vlm::{VIDEO_FRAME_PROMPT, VlmDescriptionOutcome};
use archon_policy::EffectivePolicy;
use cozo::DbInstance;

use crate::errors::VideoError;
use crate::frames::compute_frame_hash;
use crate::provenance::insert_edge;
use crate::store::{self, ChunkTimeRef, FrameDescription};

pub async fn run_frame_ocr(
    frame: &FrameDescription,
    db: &DbInstance,
    ocr_provider: &dyn OcrProvider,
    document_id: &str,
) -> Result<Option<String>, VideoError> {
    let image_path = resolve_frame_image_path(frame)?;
    let request = OcrRequest {
        file_path: image_path.display().to_string(),
        document_id: document_id.to_string(),
        ocr_run_id: format!("ocr-{}", uuid::Uuid::new_v4()),
        page_range: Some((1, 1)),
        language_hint: None,
    };
    let result = match ocr_provider.extract(request).await {
        Ok(result) => result,
        Err(error) => {
            store::update_frame_status(db, &frame.frame_id, "ocr_failed", &error.to_string())
                .map_err(store_error)?;
            return Ok(None);
        }
    };
    let text = result.full_text.trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }
    store::update_frame_ocr_text(db, &frame.frame_id, &text).map_err(store_error)?;
    write_visual_chunk(db, frame, document_id, &text, &frame.image_artifact_id)?;
    Ok(Some(text))
}

pub async fn run_frame_vlm(
    frame: &FrameDescription,
    db: &DbInstance,
    policy: &EffectivePolicy,
    document_id: &str,
) -> Result<Option<String>, VideoError> {
    let decision = policy.video_vlm_decision();
    if !decision.allowed {
        return Ok(None);
    }
    let image_bytes = tokio::fs::read(resolve_frame_image_path(frame)?)
        .await
        .map_err(|e| VideoError::FrameExtractionFailed {
            message: format!("read frame image: {e}"),
        })?;
    let policy_clone = policy.clone();
    let vlm_result = tokio::task::spawn_blocking(move || {
        archon_docs::vlm::describe_registered_image(
            &policy_clone,
            &image_bytes,
            Some(VIDEO_FRAME_PROMPT),
        )
    })
    .await
    .map_err(|e| VideoError::Store {
        message: format!("VLM worker join failed: {e}"),
    })?;
    let description = match vlm_result {
        Ok(VlmDescriptionOutcome::Described(description)) => description,
        Ok(VlmDescriptionOutcome::Disabled(_)) | Ok(VlmDescriptionOutcome::NoProvider) => {
            return Ok(None);
        }
        Err(error) => {
            store::update_frame_status(db, &frame.frame_id, "vlm_failed", &error.to_string())
                .map_err(store_error)?;
            return Ok(None);
        }
    };
    let text = description.text.trim().to_string();
    if text.is_empty() {
        return Ok(None);
    }
    store::update_frame_vlm_description(db, &frame.frame_id, &text).map_err(store_error)?;
    let artifact_id = artifact_for_type(db, document_id, "video_frame_vlm", frame)
        .unwrap_or_else(|| frame.image_artifact_id.clone());
    write_visual_chunk(db, frame, document_id, &text, &artifact_id)?;
    Ok(Some(text))
}

fn write_visual_chunk(
    db: &DbInstance,
    frame: &FrameDescription,
    document_id: &str,
    text: &str,
    artifact_id: &str,
) -> Result<String, VideoError> {
    let chunk_id = format!("chunk-frame-{}", uuid::Uuid::new_v4());
    archon_docs::store::insert_chunk(
        db,
        &ChunkArtifact {
            chunk_id: chunk_id.clone(),
            document_id: document_id.to_string(),
            artifact_id: artifact_id.to_string(),
            chunk_index: 0,
            page_start: 0,
            page_end: 0,
            content: text.to_string(),
            content_hash: sha256_str(text),
            embedding_status: "pending".into(),
        },
    )
    .map_err(store_error)?;
    store::insert_chunk_timeref(
        db,
        &ChunkTimeRef {
            chunk_id: chunk_id.clone(),
            video_id: frame.video_id.clone(),
            track_id: frame.track_id.clone(),
            timestamp_start_ms: frame.timestamp_ms,
            timestamp_end_ms: frame.timestamp_end_ms,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )?;
    insert_edge(
        db,
        &chunk_id,
        artifact_id,
        ProvenanceEdgeType::ExtractedFrom,
    )
    .map_err(store_error)?;
    Ok(chunk_id)
}

pub fn resolve_frame_image_path(frame: &FrameDescription) -> Result<PathBuf, VideoError> {
    let dir = PathBuf::from(".archon")
        .join("video-artifacts")
        .join(&frame.video_id)
        .join("frames");
    find_frame_by_hash(&dir, &frame.frame_hash).ok_or_else(|| VideoError::FrameExtractionFailed {
        message: format!(
            "frame image for {} with hash {} was not found in {}",
            frame.frame_id,
            frame.frame_hash,
            dir.display()
        ),
    })
}

fn find_frame_by_hash(dir: &Path, hash: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && compute_frame_hash(&path).ok().as_deref() == Some(hash) {
            return Some(path);
        }
    }
    None
}

fn artifact_for_type(
    db: &DbInstance,
    document_id: &str,
    artifact_type: &str,
    frame: &FrameDescription,
) -> Option<String> {
    archon_docs::store::list_artifacts_for_doc(db, document_id)
        .ok()?
        .into_iter()
        .find(|artifact| {
            artifact.artifact_type == artifact_type && artifact.content_hash == frame.frame_hash
        })
        .map(|artifact| artifact.artifact_id)
}

fn store_error(error: impl std::fmt::Display) -> VideoError {
    VideoError::Store {
        message: error.to_string(),
    }
}
