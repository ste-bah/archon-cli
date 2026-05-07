// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Pad with zeros if v.len() < target, truncate if v.len() > target.
pub fn pad_or_truncate(mut v: Vec<f32>, target: usize) -> Vec<f32> {
    if v.len() < target {
        v.resize(target, 0.0);
    } else if v.len() > target {
        v.truncate(target);
    }
    v
}
