use super::constants::{WEIGHT_MAX, WEIGHT_MIN};

// ---------------------------------------------------------------------------
// Math utilities (matching sona-utils.ts)
// ---------------------------------------------------------------------------

/// reward = quality * l_score * success_rate
pub fn calculate_reward(quality: f64, l_score: f64, success_rate: f64) -> f64 {
    quality * l_score * success_rate
}

/// gradient = (reward - 0.5) * similarity
pub fn calculate_gradient(reward: f64, similarity: f64) -> f64 {
    (reward - 0.5) * similarity
}

/// weight_change = learning_rate * gradient / (1 + regularization * importance)
/// Result clamped to [WEIGHT_MIN, WEIGHT_MAX].
pub fn calculate_weight_update(
    gradient: f64,
    learning_rate: f64,
    regularization: f64,
    importance: f64,
) -> f64 {
    let change = learning_rate * gradient / (1.0 + regularization * importance);
    change.clamp(WEIGHT_MIN, WEIGHT_MAX)
}

/// fisher_update = decay * old_importance + (1 - decay) * gradient^2
pub fn update_fisher_information(old_importance: f64, gradient: f64, decay: f64) -> f64 {
    decay * old_importance + (1.0 - decay) * gradient * gradient
}

/// Cosine similarity between two vectors. Returns 0.0 for zero-norm vectors.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();

    if norm_a < 1e-12 || norm_b < 1e-12 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

// ---------------------------------------------------------------------------
// CRC32 (polynomial 0xEDB88320)
// ---------------------------------------------------------------------------

/// Compute CRC32 checksum using polynomial 0xEDB88320.
pub fn crc32_checksum(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}
