use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::errors::VideoError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameExtractionMode {
    None,
    Interval,
    Scene,
    Hybrid,
}

impl FrameExtractionMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "interval" => Self::Interval,
            "scene" => Self::Scene,
            "hybrid" => Self::Hybrid,
            "none" | "off" | "disabled" => Self::None,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FrameExtractionOpts {
    pub mode: FrameExtractionMode,
    pub interval_secs: f64,
    pub scene_threshold: f32,
    pub max_frames: u32,
    pub ffmpeg_bin: String,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFrame {
    pub timestamp_ms: u64,
    pub timestamp_end_ms: u64,
    pub image_path: PathBuf,
    pub frame_hash: String,
    pub sequence_index: u32,
}

pub async fn extract_frames(
    video_path: &Path,
    opts: &FrameExtractionOpts,
) -> Result<Vec<ExtractedFrame>, VideoError> {
    tokio::fs::create_dir_all(&opts.output_dir)
        .await
        .map_err(|e| VideoError::FrameExtractionFailed {
            message: format!("create frame output dir: {e}"),
        })?;
    match opts.mode {
        FrameExtractionMode::None => Ok(Vec::new()),
        FrameExtractionMode::Interval => extract_mode(video_path, opts, "frame").await,
        FrameExtractionMode::Scene => extract_mode(video_path, opts, "scene").await,
        FrameExtractionMode::Hybrid => {
            let mut frames = extract_mode(video_path, opts, "frame").await?;
            frames.extend(extract_mode(video_path, opts, "scene").await?);
            frames.sort_by_key(|frame| frame.timestamp_ms);
            frames.dedup_by_key(|frame| frame.timestamp_ms);
            frames.truncate(opts.max_frames as usize);
            Ok(frames)
        }
    }
}

async fn extract_mode(
    video_path: &Path,
    opts: &FrameExtractionOpts,
    prefix: &str,
) -> Result<Vec<ExtractedFrame>, VideoError> {
    let ffmpeg_result = extract_mode_ffmpeg(video_path, opts, prefix).await;
    if !crate::opencv_frames::fallback_enabled() {
        return ffmpeg_result;
    }
    match ffmpeg_result {
        Ok(frames) if !frames.is_empty() => Ok(frames),
        Ok(frames) => {
            match crate::opencv_frames::extract_with_opencv(video_path, opts, prefix).await {
                Ok(fallback) if !fallback.is_empty() => Ok(fallback),
                _ => Ok(frames),
            }
        }
        Err(ffmpeg_error) => {
            match crate::opencv_frames::extract_with_opencv(video_path, opts, prefix).await {
                Ok(fallback) if !fallback.is_empty() => Ok(fallback),
                Ok(_) => Err(ffmpeg_error),
                Err(fallback_error) => Err(VideoError::FrameExtractionFailed {
                    message: format!("{ffmpeg_error}; OpenCV fallback failed: {fallback_error}"),
                }),
            }
        }
    }
}

async fn extract_mode_ffmpeg(
    video_path: &Path,
    opts: &FrameExtractionOpts,
    prefix: &str,
) -> Result<Vec<ExtractedFrame>, VideoError> {
    let pattern = opts.output_dir.join(format!("{prefix}_%04d.png"));
    let filter = if prefix == "scene" {
        format!("select=gt(scene\\,{})", opts.scene_threshold)
    } else {
        format!("fps=1/{}", opts.interval_secs.max(0.1))
    };
    let output = Command::new(ffmpeg_bin(&opts.ffmpeg_bin))
        .arg("-hide_banner")
        .arg("-nostdin")
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-vf")
        .arg(filter)
        .arg("-frames:v")
        .arg(opts.max_frames.to_string())
        .arg("-f")
        .arg("image2")
        .arg(&pattern)
        .output()
        .await
        .map_err(|_| VideoError::BinaryNotFound {
            name: "ffmpeg".into(),
            path: ffmpeg_bin(&opts.ffmpeg_bin),
        })?;
    if !output.status.success() {
        return Err(VideoError::FrameExtractionFailed {
            message: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    collect_frames(
        &opts.output_dir,
        prefix,
        opts.interval_secs,
        opts.max_frames,
    )
}

pub(crate) fn collect_frames(
    dir: &Path,
    prefix: &str,
    interval_secs: f64,
    max_frames: u32,
) -> Result<Vec<ExtractedFrame>, VideoError> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| VideoError::FrameExtractionFailed {
            message: format!("read frame output dir: {e}"),
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| frame_name_matches(path, prefix))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .take(max_frames as usize)
        .enumerate()
        .map(|(index, image_path)| {
            let timestamp_ms = ((index as f64 + 1.0) * interval_secs * 1000.0).round() as u64;
            Ok(ExtractedFrame {
                timestamp_ms,
                timestamp_end_ms: timestamp_ms + (interval_secs * 1000.0).round() as u64,
                frame_hash: compute_frame_hash(&image_path)?,
                image_path,
                sequence_index: (index + 1) as u32,
            })
        })
        .collect()
}

fn frame_name_matches(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".png"))
}

pub fn compute_frame_hash(image_path: &Path) -> Result<String, VideoError> {
    let bytes = std::fs::read(image_path).map_err(|e| VideoError::FrameExtractionFailed {
        message: format!("read frame image {}: {e}", image_path.display()),
    })?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn ffmpeg_bin(configured: &str) -> String {
    std::env::var("ARCHON_FFMPEG_BIN").unwrap_or_else(|_| configured.to_string())
}
