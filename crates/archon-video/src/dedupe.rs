use crate::errors::VideoError;
use crate::frames::ExtractedFrame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DedupeGroup {
    pub dedupe_group_id: String,
    pub representative: ExtractedFrame,
    pub representative_hash: u64,
    pub member_timestamps: Vec<u64>,
    pub first_timestamp_ms: u64,
    pub last_timestamp_ms: u64,
    pub frame_count: usize,
}

pub fn compute_perceptual_hash(image_bytes: &[u8]) -> Result<u64, VideoError> {
    let image = image::load_from_memory(image_bytes)
        .map_err(|e| VideoError::ImageDecodeFailed {
            message: e.to_string(),
        })?
        .grayscale()
        .resize_exact(8, 8, image::imageops::FilterType::Triangle)
        .to_luma8();
    let pixels = image.into_raw();
    let mean = pixels.iter().map(|pixel| *pixel as u32).sum::<u32>() / 64;
    Ok(pixels
        .iter()
        .enumerate()
        .fold(0_u64, |hash, (index, pixel)| {
            if *pixel as u32 > mean {
                hash | (1_u64 << index)
            } else {
                hash
            }
        }))
}

pub fn hamming_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

pub fn deduplicate_frames(
    frames: Vec<ExtractedFrame>,
    threshold: f32,
) -> Result<Vec<DedupeGroup>, VideoError> {
    let max_distance = ((1.0 - threshold.clamp(0.0, 1.0)) * 64.0).ceil() as u32;
    let mut groups: Vec<DedupeGroup> = Vec::new();
    for frame in frames {
        let bytes =
            std::fs::read(&frame.image_path).map_err(|e| VideoError::ImageDecodeFailed {
                message: format!("read frame image {}: {e}", frame.image_path.display()),
            })?;
        let hash = compute_perceptual_hash(&bytes)?;
        if let Some(group) = groups
            .iter_mut()
            .find(|group| hamming_distance(group.representative_hash, hash) <= max_distance)
        {
            group.member_timestamps.push(frame.timestamp_ms);
            group.first_timestamp_ms = group.first_timestamp_ms.min(frame.timestamp_ms);
            group.last_timestamp_ms = group.last_timestamp_ms.max(frame.timestamp_ms);
            group.frame_count += 1;
        } else {
            groups.push(DedupeGroup {
                dedupe_group_id: uuid::Uuid::new_v4().to_string(),
                representative: frame.clone(),
                representative_hash: hash,
                member_timestamps: vec![frame.timestamp_ms],
                first_timestamp_ms: frame.timestamp_ms,
                last_timestamp_ms: frame.timestamp_ms,
                frame_count: 1,
            });
        }
    }
    Ok(groups)
}
