use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use archon_video::frames::{
    FrameExtractionMode, FrameExtractionOpts, compute_frame_hash, extract_frames,
};

fn write_test_image(path: &Path, color: [u8; 3]) {
    let image = image::RgbImage::from_pixel(8, 8, image::Rgb(color));
    image.save(path).unwrap();
}

fn write_mock_ffmpeg(dir: &Path, source_image: &Path) -> PathBuf {
    let script = dir.join("ffmpeg-mock.sh");
    let body = format!(
        r#"#!/bin/sh
for arg do out="$arg"; done
dir=$(dirname "$out")
base=$(basename "$out")
i=1
while [ "$i" -le 3 ]; do
  number=$(printf "%04d" "$i")
  name=$(printf "%s" "$base" | sed "s/%04d/$number/")
  cp '{}' "$dir/$name"
  i=$((i + 1))
done
"#,
        source_image.display()
    );
    std::fs::write(&script, body).unwrap();
    let mut perms = std::fs::metadata(&script).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script, perms).unwrap();
    script
}

#[tokio::test]
async fn interval_frame_extraction_respects_max_frames() {
    let dir = tempfile::tempdir().unwrap();
    let source_image = dir.path().join("source.png");
    write_test_image(&source_image, [255, 0, 0]);
    let ffmpeg = write_mock_ffmpeg(dir.path(), &source_image);
    let output_dir = dir.path().join("frames");

    let frames = extract_frames(
        Path::new("fixture.mp4"),
        &FrameExtractionOpts {
            mode: FrameExtractionMode::Interval,
            interval_secs: 2.0,
            scene_threshold: 0.35,
            max_frames: 2,
            ffmpeg_bin: ffmpeg.display().to_string(),
            output_dir,
        },
    )
    .await
    .unwrap();

    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|frame| frame.timestamp_ms > 0));
    assert!(frames.iter().all(|frame| frame.image_path.exists()));
    assert!(frames.iter().all(|frame| frame.frame_hash.len() == 64));
}

#[test]
fn frame_hash_is_sha256_hex() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("source.png");
    write_test_image(&image, [0, 255, 0]);

    let hash = compute_frame_hash(&image).unwrap();

    assert_eq!(hash.len(), 64);
}
