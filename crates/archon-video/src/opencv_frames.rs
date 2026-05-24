use std::path::Path;

use tokio::process::Command;

use crate::errors::VideoError;
use crate::frames::{ExtractedFrame, FrameExtractionOpts, collect_frames};

pub(crate) fn fallback_enabled() -> bool {
    std::env::var("ARCHON_VIDEO_FRAME_FALLBACK")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

pub(crate) async fn extract_with_opencv(
    video_path: &Path,
    opts: &FrameExtractionOpts,
    prefix: &str,
) -> Result<Vec<ExtractedFrame>, VideoError> {
    let output = Command::new(python_bin())
        .arg("-c")
        .arg(OPENCV_EXTRACTOR)
        .arg(video_path)
        .arg(&opts.output_dir)
        .arg(prefix)
        .arg(opts.interval_secs.max(0.1).to_string())
        .arg(opts.max_frames.to_string())
        .output()
        .await
        .map_err(|e| VideoError::FrameExtractionFailed {
            message: format!("start Python OpenCV frame fallback: {e}"),
        })?;

    if !output.status.success() {
        return Err(VideoError::FrameExtractionFailed {
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    collect_frames(
        &opts.output_dir,
        prefix,
        opts.interval_secs,
        opts.max_frames,
    )
}

fn python_bin() -> String {
    std::env::var("ARCHON_VIDEO_OPENCV_PYTHON")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "python3".into())
}

const OPENCV_EXTRACTOR: &str = r#"
import sys
from pathlib import Path

try:
    import cv2
except Exception as exc:
    print(f"opencv-python is not available: {exc}", file=sys.stderr)
    sys.exit(2)

video_path = Path(sys.argv[1])
output_dir = Path(sys.argv[2])
prefix = sys.argv[3]
interval_secs = max(float(sys.argv[4]), 0.1)
max_frames = max(int(sys.argv[5]), 0)
output_dir.mkdir(parents=True, exist_ok=True)

cap = cv2.VideoCapture(str(video_path))
if not cap.isOpened():
    print(f"OpenCV could not open video: {video_path}", file=sys.stderr)
    sys.exit(3)

fps = cap.get(cv2.CAP_PROP_FPS)
if not fps or fps <= 0:
    fps = 30.0
step_frames = max(1, int(round(interval_secs * fps)))

written = 0
target_frame = 0
while written < max_frames:
    cap.set(cv2.CAP_PROP_POS_FRAMES, target_frame)
    ok, frame = cap.read()
    if not ok:
        break
    out = output_dir / f"{prefix}_{written + 1:04d}.png"
    if cv2.imwrite(str(out), frame):
        written += 1
    target_frame += step_frames

cap.release()
if written == 0:
    print("OpenCV frame fallback produced no frames", file=sys.stderr)
    sys.exit(4)
print(written)
"#;
