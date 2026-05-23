use std::path::{Path, PathBuf};

use archon_policy::EffectivePolicy;
use cozo::DbInstance;

use crate::dedupe::deduplicate_frames;
use crate::frame_persist::persist_frame_groups;
use crate::frames::{FrameExtractionMode, FrameExtractionOpts, extract_frames};
use crate::store::list_frame_descriptions_for_video;
use crate::visual::{run_frame_ocr, run_frame_vlm};

pub struct FramePipelineInput<'a> {
    pub frames_mode_override: Option<&'a str>,
    pub local_video_path: Option<&'a Path>,
    pub video_id: &'a str,
    pub document_id: &'a str,
    pub source_artifact_id: &'a str,
    pub created_at: &'a str,
}

pub async fn process_frame_pipeline(
    db: &DbInstance,
    policy: &EffectivePolicy,
    input: FramePipelineInput<'_>,
    warnings: &mut Vec<String>,
) {
    let mode_text = input
        .frames_mode_override
        .unwrap_or(policy.video.frames.mode.as_str());
    let mode = FrameExtractionMode::parse(mode_text);
    if mode == FrameExtractionMode::None {
        return;
    }
    let Some(video_path) = input.local_video_path else {
        warnings.push("frame extraction skipped: no local video file available".into());
        return;
    };
    if !video_path.exists() {
        warnings.push(format!(
            "frame extraction skipped: local video file not found at {}",
            video_path.display()
        ));
        return;
    }
    let extraction =
        extract_frames(video_path, &extraction_opts(policy, input.video_id, mode)).await;
    let groups = match extraction
        .and_then(|frames| deduplicate_frames(frames, policy.video.dedupe_threshold))
    {
        Ok(groups) => groups,
        Err(error) => {
            warnings.push(format!("frame extraction failed: {error}"));
            return;
        }
    };
    if let Err(error) = persist_frame_groups(
        db,
        input.document_id,
        input.video_id,
        input.source_artifact_id,
        &groups,
        &policy.video,
        input.created_at,
    ) {
        warnings.push(format!("frame persistence failed: {error}"));
        return;
    }
    run_visual_steps(db, policy, &input, warnings).await;
}

async fn run_visual_steps(
    db: &DbInstance,
    policy: &EffectivePolicy,
    input: &FramePipelineInput<'_>,
    warnings: &mut Vec<String>,
) {
    let frames = match list_frame_descriptions_for_video(db, input.video_id) {
        Ok(frames) => frames,
        Err(error) => {
            warnings.push(format!("frame visual steps skipped: {error}"));
            return;
        }
    };
    if policy.video.frames.ocr {
        if let Some(provider) = archon_docs::ocr::provider::get_provider() {
            for frame in &frames {
                if let Err(error) =
                    run_frame_ocr(frame, db, provider.as_ref(), input.document_id).await
                {
                    warnings.push(format!("frame OCR failed for {}: {error}", frame.frame_id));
                }
            }
        } else {
            warnings.push("frame OCR skipped: OCR provider not configured".into());
        }
    }
    if policy.video.frames.vlm {
        for frame in &frames {
            if let Err(error) = run_frame_vlm(frame, db, policy, input.document_id).await {
                warnings.push(format!("frame VLM failed for {}: {error}", frame.frame_id));
            }
        }
    }
}

fn extraction_opts(
    policy: &EffectivePolicy,
    video_id: &str,
    mode: FrameExtractionMode,
) -> FrameExtractionOpts {
    FrameExtractionOpts {
        mode,
        interval_secs: policy.video.frame_interval_secs.max(1) as f64,
        scene_threshold: policy.video.scene_change_threshold,
        max_frames: policy.video.max_frames,
        ffmpeg_bin: "ffmpeg".into(),
        output_dir: PathBuf::from(".archon")
            .join("video-artifacts")
            .join(video_id)
            .join("frames"),
    }
}
