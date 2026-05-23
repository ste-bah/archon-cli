use std::path::{Path, PathBuf};

use archon_video::dedupe::{deduplicate_frames, hamming_distance};
use archon_video::frames::{ExtractedFrame, compute_frame_hash};

fn write_pattern(path: &Path, variant: u8) {
    let mut image = image::RgbImage::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            let bright = if variant == 0 { x < 4 } else { y < 4 };
            let value = if bright { 255 } else { 0 };
            image.put_pixel(x, y, image::Rgb([value, value, value]));
        }
    }
    image.save(path).unwrap();
}

fn frame(path: PathBuf, timestamp_ms: u64, sequence_index: u32) -> ExtractedFrame {
    ExtractedFrame {
        timestamp_ms,
        timestamp_end_ms: timestamp_ms + 500,
        frame_hash: compute_frame_hash(&path).unwrap(),
        image_path: path,
        sequence_index,
    }
}

#[test]
fn identical_frames_collapse_to_one_dedupe_group() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("same.png");
    write_pattern(&image, 0);
    let frames = (0..10)
        .map(|index| frame(image.clone(), 1_000 + index * 500, index as u32))
        .collect();

    let groups = deduplicate_frames(frames, 0.94).unwrap();

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].frame_count, 10);
    assert_eq!(groups[0].first_timestamp_ms, 1_000);
    assert_eq!(groups[0].last_timestamp_ms, 5_500);
    assert_eq!(groups[0].member_timestamps.len(), 10);
}

#[test]
fn distinct_frames_create_distinct_groups() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.png");
    let second = dir.path().join("second.png");
    write_pattern(&first, 0);
    write_pattern(&second, 1);
    let frames = vec![frame(first, 1_000, 1), frame(second, 2_000, 2)];

    let groups = deduplicate_frames(frames, 0.94).unwrap();

    assert_eq!(groups.len(), 2);
    assert!(hamming_distance(groups[0].representative_hash, groups[1].representative_hash) > 4);
}
