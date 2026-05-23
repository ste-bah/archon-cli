use std::path::PathBuf;

use archon_docs::hash::{sha256_hex, sha256_str};
use archon_docs::models::{DocumentStatus, ProcessingJob, ProvenanceEdgeType, SourceDocument};
use archon_policy::EffectivePolicy;
use cozo::DbInstance;

use crate::asr::{
    AsrOptions, AsrProvider, DiarizationProvider, apply_diarization, enforce_monotonic_boundaries,
    select_asr_provider, select_diarizer_provider,
};
use crate::chunk_writer::{
    TranscriptChunkInput, write_transcript_artifact, write_transcript_chunk,
};
use crate::errors::VideoError;
use crate::frame_pipeline::{FramePipelineInput, process_frame_pipeline};
use crate::ingest_media::{acquire_media_if_needed, asr_audio_bytes, successful_video_by_hash};
use crate::metadata::{MetadataOpts, VideoMetadata, extract_metadata};
use crate::provenance::insert_edge;
use crate::schema::create_video_schema;
use crate::source::{ResolveOpts, resolve_source};
use crate::store::{self, TranscriptSegment as StoredSegment, VideoSource, VideoTrack};
use crate::transcript::parse_transcript;

#[derive(Debug, Clone)]
pub struct IngestOpts {
    pub source: String,
    pub transcript_path: Option<PathBuf>,
    pub metadata_only: bool,
    pub frames_mode: Option<String>,
    pub asr_provider: Option<String>,
    pub vlm: bool,
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoIngestResult {
    pub video_id: String,
    pub document_id: String,
    pub chunk_count: usize,
    pub was_new: bool,
    pub warnings: Vec<String>,
}

pub async fn ingest_video(
    opts: IngestOpts,
    policy: &EffectivePolicy,
    db: &DbInstance,
) -> Result<VideoIngestResult, VideoError> {
    ingest_video_with_providers(opts, policy, db, None, None).await
}

pub async fn ingest_video_with_asr_provider(
    opts: IngestOpts,
    policy: &EffectivePolicy,
    db: &DbInstance,
    asr_provider: Option<&dyn AsrProvider>,
) -> Result<VideoIngestResult, VideoError> {
    ingest_video_with_providers(opts, policy, db, asr_provider, None).await
}

pub async fn ingest_video_with_providers(
    opts: IngestOpts,
    policy: &EffectivePolicy,
    db: &DbInstance,
    asr_provider: Option<&dyn AsrProvider>,
    diarizer_provider: Option<&dyn DiarizationProvider>,
) -> Result<VideoIngestResult, VideoError> {
    let mut effective_policy = policy.clone();
    if opts.vlm {
        effective_policy.video.frames.vlm = true;
    }
    let policy = &effective_policy;

    archon_docs::schema::ensure_doc_schema(db)?;
    create_video_schema(db)?;

    let resolve_opts = ResolveOpts {
        transcript_path: opts.transcript_path.clone(),
        metadata_only: opts.metadata_only,
        prefer_caption: false,
    };
    let mut resolution = resolve_source(&opts.source, &resolve_opts, policy)?;
    if let Some(media) = acquire_media_if_needed(&opts, policy, &resolution).await? {
        resolution.local_path = Some(media.local_path);
    }
    let metadata = maybe_extract_metadata(&resolution).await?;
    let transcript_bytes = read_transcript_bytes(&opts).await?;
    let source_hash = compute_source_hash(&opts.source, transcript_bytes.as_deref());
    if let Some(existing) = successful_video_by_hash(db, &source_hash)? {
        return Ok(VideoIngestResult {
            video_id: existing.video_id,
            document_id: existing.document_id,
            chunk_count: 0,
            was_new: false,
            warnings: Vec::new(),
        });
    }

    let video_id = format!("video-{}", uuid::Uuid::new_v4());
    let document_id = format!("doc-{video_id}");
    let now = now();
    insert_source_rows(
        db,
        &opts,
        &resolution,
        &video_id,
        &document_id,
        &source_hash,
        &metadata,
        &now,
    )?;

    let segment_plan = resolve_segment_plan(
        &opts,
        policy,
        transcript_bytes,
        resolution.local_path.as_deref(),
        asr_provider,
        diarizer_provider,
    )
    .await?;
    if segment_plan.segments.is_empty() {
        store::update_video_status(db, &video_id, "NoEvidenceExtracted", &now)?;
        return Err(VideoError::NoEvidenceExtracted);
    }

    let track_id = format!("track-{}", uuid::Uuid::new_v4());
    store::insert_video_track(
        db,
        &VideoTrack {
            track_id: track_id.clone(),
            video_id: video_id.clone(),
            track_kind: "transcript".into(),
            provider: segment_plan.provider.clone(),
            model: String::new(),
            status: "running".into(),
            warning_count: 0,
            error_count: 0,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    )?;

    let transcript_text = segment_plan
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let artifact_id =
        write_transcript_artifact(db, &document_id, &transcript_text, &now).map_err(store_error)?;
    insert_edge(
        db,
        &artifact_id,
        &document_id,
        ProvenanceEdgeType::DerivedFrom,
    )
    .map_err(store_error)?;

    for (index, segment) in segment_plan.segments.iter().enumerate() {
        let chunk_id = write_transcript_chunk(
            db,
            &TranscriptChunkInput {
                document_id: &document_id,
                artifact_id: &artifact_id,
                video_id: &video_id,
                track_id: &track_id,
                chunk_index: index as u32,
                segment,
                created_at: &now,
            },
        )
        .map_err(store_error)?;
        store::insert_transcript_segment(
            db,
            &StoredSegment {
                segment_id: format!("segment-{}", uuid::Uuid::new_v4()),
                video_id: video_id.clone(),
                track_id: track_id.clone(),
                start_ms: segment.start_ms as i64,
                end_ms: segment.end_ms as i64,
                speaker: segment.speaker.clone().unwrap_or_default(),
                text: segment.text.clone(),
                confidence: segment.confidence.unwrap_or(-1.0) as f64,
                source_method: segment_plan.source_method.clone(),
                chunk_id: chunk_id.clone(),
                created_at: now.clone(),
            },
        )?;
        insert_edge(
            db,
            &chunk_id,
            &artifact_id,
            ProvenanceEdgeType::ExtractedFrom,
        )
        .map_err(store_error)?;
    }

    let mut warnings = segment_plan.warnings;
    process_frame_pipeline(
        db,
        policy,
        FramePipelineInput {
            frames_mode_override: opts.frames_mode.as_deref(),
            local_video_path: resolution.local_path.as_deref(),
            video_id: &video_id,
            document_id: &document_id,
            source_artifact_id: &artifact_id,
            created_at: &now,
        },
        &mut warnings,
    )
    .await;

    store::update_track_status(db, &track_id, "success", warnings.len() as i64, 0, &now)?;
    store::update_video_status(db, &video_id, "success", &now)?;
    archon_docs::store::update_doc_status(db, &document_id, &DocumentStatus::Ingested)
        .map_err(store_error)?;

    Ok(VideoIngestResult {
        video_id,
        document_id,
        chunk_count: segment_plan.segments.len(),
        was_new: true,
        warnings,
    })
}

async fn read_transcript_bytes(opts: &IngestOpts) -> Result<Option<Vec<u8>>, VideoError> {
    let Some(path) = &opts.transcript_path else {
        return Ok(None);
    };
    tokio::fs::read(path)
        .await
        .map(Some)
        .map_err(|e| VideoError::SourceNotFound {
            path: format!("{} ({e})", path.display()),
        })
}

struct SegmentPlan {
    segments: Vec<crate::transcript::TranscriptSegment>,
    warnings: Vec<String>,
    source_method: String,
    provider: String,
}

async fn resolve_segment_plan(
    opts: &IngestOpts,
    policy: &EffectivePolicy,
    transcript_bytes: Option<Vec<u8>>,
    media_path: Option<&std::path::Path>,
    asr_provider: Option<&dyn AsrProvider>,
    diarizer_provider: Option<&dyn DiarizationProvider>,
) -> Result<SegmentPlan, VideoError> {
    if let Some(bytes) = transcript_bytes {
        let parsed = parse_transcript(&bytes, None)?;
        return Ok(SegmentPlan {
            segments: parsed.segments,
            warnings: parsed.warnings,
            source_method: "user_transcript".into(),
            provider: "user_transcript".into(),
        });
    }
    if opts.metadata_only {
        return Ok(SegmentPlan {
            segments: Vec::new(),
            warnings: vec!["metadata-only ingest produced no transcript segments".into()],
            source_method: "none".into(),
            provider: "none".into(),
        });
    }
    let asr_opts = AsrOptions::from(&policy.video);
    let audio_bytes = asr_audio_bytes(media_path, asr_provider.is_some()).await?;
    if let Some(provider) = asr_provider {
        return build_asr_segment_plan(
            provider,
            &asr_opts,
            policy,
            diarizer_provider,
            &audio_bytes,
        )
        .await;
    }
    let provider = select_asr_provider(&policy.video);
    build_asr_segment_plan(
        provider.as_ref(),
        &asr_opts,
        policy,
        diarizer_provider,
        &audio_bytes,
    )
    .await
}

async fn build_asr_segment_plan(
    provider: &dyn AsrProvider,
    asr_opts: &AsrOptions,
    policy: &EffectivePolicy,
    diarizer_provider: Option<&dyn DiarizationProvider>,
    audio_bytes: &[u8],
) -> Result<SegmentPlan, VideoError> {
    let mut segments = provider.transcribe(audio_bytes, asr_opts).await?;
    let mut warnings = Vec::new();
    if asr_opts.vad_stable_timestamps {
        enforce_monotonic_boundaries(&mut segments);
    }
    if asr_opts.diarization {
        let selected;
        let diarizer = match diarizer_provider {
            Some(provider) => provider,
            None => {
                selected = select_diarizer_provider(&policy.video);
                selected.as_ref()
            }
        };
        let (with_speakers, diarization_warnings) =
            apply_diarization(segments, audio_bytes, diarizer).await;
        segments = with_speakers;
        warnings.extend(diarization_warnings);
    }
    Ok(SegmentPlan {
        segments,
        warnings,
        source_method: "local_asr".into(),
        provider: provider.provider_name().into(),
    })
}

fn insert_source_rows(
    db: &DbInstance,
    opts: &IngestOpts,
    resolution: &crate::source::VideoSourceResolution,
    video_id: &str,
    document_id: &str,
    source_hash: &str,
    metadata: &VideoMetadata,
    now: &str,
) -> Result<(), VideoError> {
    archon_docs::store::insert_doc_source(
        db,
        &SourceDocument {
            document_id: document_id.to_string(),
            source_path: opts.source.clone(),
            media_type: media_type_for(&resolution.source_kind).into(),
            content_hash: source_hash.to_string(),
            discovered_at: now.to_string(),
            status: DocumentStatus::Ingesting,
        },
    )
    .map_err(store_error)?;
    archon_docs::store::insert_processing_job(
        db,
        &ProcessingJob {
            job_id: format!("job-{}", uuid::Uuid::new_v4()),
            document_id: document_id.to_string(),
            job_type: "video_ingest".into(),
            status: "running".into(),
            started_at: now.to_string(),
            completed_at: None,
            error_message: None,
        },
    )
    .map_err(store_error)?;
    store::insert_video_source(
        db,
        &VideoSource {
            video_id: video_id.to_string(),
            document_id: document_id.to_string(),
            source_kind: resolution.source_kind.to_string(),
            source_url: resolution.source_url.clone(),
            local_path: resolution
                .local_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            title: metadata
                .title
                .clone()
                .unwrap_or_else(|| title_for_source(&opts.source)),
            channel_or_author: metadata.channel_or_author.clone().unwrap_or_default(),
            duration_ms: metadata.duration_ms.unwrap_or_default() as i64,
            published_at: metadata.published_at.clone().unwrap_or_default(),
            license: String::new(),
            source_hash: source_hash.to_string(),
            ingest_status: "running".into(),
            policy_snapshot_json: resolution.policy_snapshot_json.clone(),
            created_at: now.to_string(),
            updated_at: now.to_string(),
        },
    )?;
    Ok(())
}

async fn maybe_extract_metadata(
    resolution: &crate::source::VideoSourceResolution,
) -> Result<VideoMetadata, VideoError> {
    let Some(path) = &resolution.local_path else {
        return Ok(VideoMetadata::default());
    };
    if !path.exists() {
        return Ok(VideoMetadata::default());
    }
    extract_metadata(&path.display().to_string(), &MetadataOpts::default()).await
}

fn media_type_for(kind: &crate::source::VideoSourceKind) -> &'static str {
    match kind {
        crate::source::VideoSourceKind::YouTube => "video/youtube",
        crate::source::VideoSourceKind::TranscriptOnly => "text/vtt",
        _ => "video/mp4",
    }
}

fn title_for_source(source: &str) -> String {
    PathBuf::from(source)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(source)
        .to_string()
}

fn compute_source_hash(source: &str, transcript_bytes: Option<&[u8]>) -> String {
    let transcript_hash = transcript_bytes.map(sha256_hex).unwrap_or_default();
    sha256_str(&format!("{source}\n{transcript_hash}"))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn store_error(error: impl std::fmt::Display) -> VideoError {
    VideoError::Store {
        message: error.to_string(),
    }
}
