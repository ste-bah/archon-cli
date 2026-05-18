use sha2::{Digest, Sha256};

use crate::schema::VECTOR_DIM;

/// Deterministic lexical feature hashing for cheap constellation drift checks.
///
/// This is deliberately not a semantic embedding model; user-facing surfaces
/// label it as a lexical feature space.
pub fn text_vector(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0; VECTOR_DIM];
    for token in tokenize(text) {
        let digest = Sha256::digest(token.as_bytes());
        let idx = ((digest[0] as usize) << 8 | digest[1] as usize) % VECTOR_DIM;
        let sign = if digest[2] % 2 == 0 { 1.0 } else { -1.0 };
        vector[idx] += sign * (1.0 + (token.len() as f32).ln());
    }
    normalize(vector)
}

pub fn centroid_vector(texts: &[String]) -> Option<Vec<f32>> {
    if texts.is_empty() {
        return None;
    }
    let mut sum = vec![0.0; VECTOR_DIM];
    for text in texts {
        for (idx, value) in text_vector(text).iter().enumerate() {
            sum[idx] += value;
        }
    }
    Some(normalize(sum))
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a
        .iter()
        .zip(b.iter())
        .map(|(left, right)| left * right)
        .sum();
    dot.clamp(-1.0, 1.0) as f64
}

fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|ch: char| !ch.is_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|token| !token.is_empty())
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_vectors_are_deterministic() {
        assert_eq!(
            text_vector("accepted secure patch"),
            text_vector("accepted secure patch")
        );
    }

    #[test]
    fn centroid_is_normalized() {
        let vector = centroid_vector(&[
            "secure tested patch".to_string(),
            "tested permission boundary".to_string(),
        ])
        .unwrap();
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.0001);
    }

    #[test]
    fn cosine_similarity_rewards_same_text() {
        let vector = text_vector("game theory incentive mechanism");
        assert!(cosine_similarity(&vector, &vector) > 0.99);
    }
}
