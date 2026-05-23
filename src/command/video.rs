use std::path::Path;

use anyhow::{Result, bail};
use archon_docs::vlm::factory::{self as vlm_factory, VlmProviderInitStatus};
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cli_args::VideoAction;

pub async fn handle_video_command(action: VideoAction) -> Result<()> {
    let db = open_db()?;
    let policy = std::env::current_dir()
        .ok()
        .and_then(|cwd| archon_policy::load_effective_policy(&cwd).ok())
        .unwrap_or_default();
    match action {
        VideoAction::Ingest {
            source,
            transcript,
            frames,
            asr,
            vlm,
            metadata_only,
            yes,
        } => {
            let mut policy = policy.clone();
            if vlm {
                policy.video.frames.vlm = true;
            }
            let vlm_report = configure_video_vlm_if_needed(&policy);
            let result = archon_video::ingest::ingest_video(
                archon_video::ingest::IngestOpts {
                    source,
                    transcript_path: transcript,
                    metadata_only,
                    frames_mode: frames,
                    asr_provider: asr,
                    vlm,
                    yes,
                },
                &policy,
                &db,
            )
            .await?;
            if result.was_new {
                println!(
                    "Ingested video: {} ({} chunk(s))",
                    result.video_id, result.chunk_count
                );
            } else {
                println!("Skipped duplicate video: {}", result.video_id);
            }
            for warning in result.warnings {
                println!("Warning: {warning}");
            }
            print_vlm_init_warning_if_needed(&vlm_report);
        }
        VideoAction::Status | VideoAction::List => list_videos(&db)?,
        VideoAction::Inspect { video_id } => inspect_video(&db, &video_id)?,
        VideoAction::Frames { video_id } => list_frames(&db, &video_id)?,
        VideoAction::Transcript { video_id, format } => export_transcript(&db, &video_id, &format)?,
        VideoAction::Summary { video_id } => show_summary(&db, &video_id)?,
        VideoAction::Delete { video_id, yes } => {
            crate::command::video_delete::delete_video(&db, &video_id, yes)?
        }
        VideoAction::Reprocess {
            video_id,
            transcript,
            frames,
            ocr,
            vlm,
            asr,
            summary,
        } => {
            let mut policy = policy.clone();
            if ocr {
                policy.video.frames.ocr = true;
            }
            if vlm {
                policy.video.frames.vlm = true;
            }
            let vlm_report = configure_video_vlm_if_needed(&policy);
            reprocess_video(
                &db,
                &policy,
                &vlm_report,
                &video_id,
                transcript,
                frames,
                ocr,
                vlm,
                asr,
                summary,
            )
            .await?
        }
    }
    Ok(())
}

fn configure_video_vlm_if_needed(
    policy: &archon_policy::EffectivePolicy,
) -> Option<vlm_factory::VlmProviderInitReport> {
    policy
        .video
        .frames
        .vlm
        .then(|| vlm_factory::configure_registered_provider(policy))
}

fn print_vlm_init_warning_if_needed(report: &Option<vlm_factory::VlmProviderInitReport>) {
    if let Some(report) = report {
        if matches!(report.status, VlmProviderInitStatus::Skipped) {
            println!("Warning: video frame VLM unavailable: {}", report.message);
        }
    }
}

async fn reprocess_video(
    db: &DbInstance,
    policy: &archon_policy::EffectivePolicy,
    vlm_report: &Option<vlm_factory::VlmProviderInitReport>,
    video_id: &str,
    transcript: bool,
    frames: bool,
    ocr: bool,
    vlm: bool,
    asr: bool,
    summary: bool,
) -> Result<()> {
    if transcript || asr || summary || (!frames && !ocr && !vlm) {
        bail!(
            "video reprocess currently supports frame evidence only; use `archon video reprocess {video_id} --frames`"
        );
    }
    let Some(source) = archon_video::store::get_video_source(db, video_id)? else {
        bail!("video not found: {video_id}");
    };
    if source.local_path.trim().is_empty() {
        bail!("video {video_id} has no local media path to reprocess");
    }
    let artifact_id = source_artifact_for_frames(db, &source.document_id)?;
    let existing = archon_video::store::list_frame_descriptions_for_video(db, video_id)?;
    if !existing.is_empty() {
        let mut warnings = Vec::new();
        archon_video::frame_pipeline::process_existing_frame_visuals(
            db,
            policy,
            video_id,
            &source.document_id,
            &mut warnings,
        )
        .await;
        println!(
            "Reprocessed video frame visuals: {video_id} ({} existing frame artifact(s))",
            existing.len()
        );
        for warning in warnings {
            println!("Warning: {warning}");
        }
        print_vlm_init_warning_if_needed(vlm_report);
        return Ok(());
    }
    let mut warnings = Vec::new();
    archon_video::frame_pipeline::process_frame_pipeline(
        db,
        policy,
        archon_video::frame_pipeline::FramePipelineInput {
            frames_mode_override: Some(policy.video.frames.mode.as_str()),
            local_video_path: Some(Path::new(&source.local_path)),
            video_id,
            document_id: &source.document_id,
            source_artifact_id: &artifact_id,
            created_at: &chrono::Utc::now().to_rfc3339(),
        },
        &mut warnings,
    )
    .await;
    let count = archon_video::store::list_frame_descriptions_for_video(db, video_id)?.len();
    println!("Reprocessed video frames: {video_id} ({count} frame artifact(s))");
    for warning in warnings {
        println!("Warning: {warning}");
    }
    print_vlm_init_warning_if_needed(vlm_report);
    Ok(())
}

fn source_artifact_for_frames(db: &DbInstance, document_id: &str) -> Result<String> {
    let artifacts = archon_docs::store::list_artifacts_for_doc(db, document_id)?;
    artifacts
        .iter()
        .find(|artifact| {
            artifact.artifact_type == archon_video::chunk_writer::ARTIFACT_TYPE_VIDEO_TRANSCRIPT
        })
        .or_else(|| artifacts.first())
        .map(|artifact| artifact.artifact_id.clone())
        .ok_or_else(|| anyhow::anyhow!("video document {document_id} has no source artifact"))
}

fn list_frames(db: &DbInstance, video_id: &str) -> Result<()> {
    let frames = archon_video::store::list_frame_descriptions_for_video(db, video_id)?;
    if frames.is_empty() {
        println!("No frame artifacts found for video {video_id}");
        return Ok(());
    }
    for frame in frames {
        println!(
            "{}  {}-{}ms  {}  {}",
            frame.frame_id,
            frame.timestamp_ms,
            frame.timestamp_end_ms,
            frame.status,
            frame.frame_hash
        );
    }
    Ok(())
}

fn export_transcript(db: &DbInstance, video_id: &str, format: &str) -> Result<()> {
    let segments = archon_video::store::get_transcript_segments_for_video(db, video_id)?;
    if segments.is_empty() {
        println!("No transcript found for video {video_id}");
        return Ok(());
    }
    let transcript_segments: Vec<_> = segments
        .into_iter()
        .map(|segment| archon_video::transcript::TranscriptSegment {
            start_ms: segment.start_ms.max(0) as u64,
            end_ms: segment.end_ms.max(0) as u64,
            text: segment.text,
            confidence: (segment.confidence >= 0.0).then_some(segment.confidence as f32),
            speaker: (!segment.speaker.is_empty()).then_some(segment.speaker),
        })
        .collect();
    let output = match format {
        "vtt" => archon_video::transcript::export_to_vtt(&transcript_segments),
        "srt" => archon_video::transcript::export_to_srt(&transcript_segments),
        "txt" | "" => archon_video::transcript::export_to_txt(&transcript_segments),
        other => anyhow::bail!("unsupported transcript format '{other}' (use txt, srt, or vtt)"),
    };
    println!("{output}");
    Ok(())
}

fn show_summary(db: &DbInstance, video_id: &str) -> Result<()> {
    let Some(source) = archon_video::store::get_video_source(db, video_id)? else {
        anyhow::bail!("video not found: {video_id}");
    };
    let artifacts = archon_docs::store::list_artifacts_for_doc(db, &source.document_id)?;
    let Some(summary_artifact) = artifacts
        .iter()
        .find(|artifact| artifact.artifact_type == "video_summary")
    else {
        println!("No summary found for video {video_id}");
        return Ok(());
    };
    let chunks = archon_docs::store::list_chunks_for_doc(db, &source.document_id)?;
    for chunk in chunks
        .iter()
        .filter(|chunk| chunk.artifact_id == summary_artifact.artifact_id)
    {
        println!("{}", chunk.content);
    }
    Ok(())
}

fn open_db() -> Result<DbInstance> {
    let db = crate::command::store_paths::open_evidence_db("video", &["ARCHON_VIDEO_DB_PATH"])?;
    archon_docs::schema::ensure_doc_schema(&db)?;
    archon_video::schema::create_video_schema(&db)?;
    Ok(db)
}

fn list_videos(db: &DbInstance) -> Result<()> {
    let sources = archon_video::store::list_video_sources(db)?;
    if sources.is_empty() {
        println!("No video sources ingested.");
        return Ok(());
    }
    for source in sources {
        println!(
            "{}  {}  {}  {}",
            source.video_id, source.ingest_status, source.source_kind, source.source_url
        );
    }
    Ok(())
}

fn inspect_video(db: &DbInstance, video_id: &str) -> Result<()> {
    let Some(source) = archon_video::store::get_video_source(db, video_id)? else {
        anyhow::bail!("video not found: {video_id}");
    };
    println!("Video: {}", source.video_id);
    println!("Document: {}", source.document_id);
    println!("Source kind: {}", source.source_kind);
    println!("Source: {}", source.source_url);
    println!("Status: {}", source.ingest_status);
    println!(
        "Tracks: {}",
        count_by_video(db, "video_tracks", "track_id", video_id)?
    );
    println!(
        "Segments: {}",
        count_by_video(db, "video_transcript_segments", "segment_id", video_id)?
    );
    println!(
        "Time refs: {}",
        count_by_video(db, "video_chunk_timeref", "chunk_id", video_id)?
    );
    println!(
        "Frames: {}",
        count_by_video(db, "video_frame_descriptions", "frame_id", video_id)?
    );
    println!(
        "Doc chunks: {}",
        archon_docs::store::list_chunks_for_doc(db, &source.document_id)?.len()
    );
    println!(
        "Provenance edges: {}",
        count_doc_provenance_edges(db, &source.document_id)?
    );
    Ok(())
}

fn count_by_video(db: &DbInstance, relation: &str, key: &str, video_id: &str) -> Result<i64> {
    let script = format!("?[count(id)] := *{relation}{{{key}: id, video_id}}, video_id = $vid");
    let mut params = std::collections::BTreeMap::new();
    params.insert("vid".into(), DataValue::from(video_id));
    let result = db
        .run_script(&script, params, ScriptMutability::Immutable)
        .map_err(|e| anyhow::anyhow!("count {relation} rows for {video_id}: {e}"))?;
    Ok(result.rows[0][0].get_int().unwrap_or(0))
}

fn count_doc_provenance_edges(db: &DbInstance, document_id: &str) -> Result<usize> {
    let artifacts = archon_docs::store::list_artifacts_for_doc(db, document_id)?;
    let mut count = 0;
    for artifact in artifacts {
        count += archon_docs::store::list_provenance_from(db, &artifact.artifact_id)?.len();
        count += archon_docs::store::list_provenance_to(db, &artifact.artifact_id)?.len();
    }
    Ok(count)
}
