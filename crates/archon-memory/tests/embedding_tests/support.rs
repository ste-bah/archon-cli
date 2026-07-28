use super::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deterministic pseudo-embedding (not random, reproducible).
pub(super) fn synthetic_embedding(dim: usize, seed: usize) -> Vec<f32> {
    (0..dim).map(|i| ((i + seed) as f32 * 0.1).sin()).collect()
}

/// A trivial provider that returns fixed-dimension synthetic embeddings.
pub(super) struct MockProvider {
    dim: usize,
}

impl MockProvider {
    pub(super) fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl EmbeddingProvider for MockProvider {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts
            .iter()
            .enumerate()
            .map(|(i, _)| synthetic_embedding(self.dim, i))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

pub(super) struct MismatchedProvider {
    pub(super) declared_dim: usize,
    pub(super) actual_dim: usize,
}

impl EmbeddingProvider for MismatchedProvider {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts
            .iter()
            .enumerate()
            .map(|(i, _)| synthetic_embedding(self.actual_dim, i))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.declared_dim
    }
}

// ---------------------------------------------------------------------------
