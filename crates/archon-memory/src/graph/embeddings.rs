use std::sync::Arc;

use super::{MIN_EMBED_CHARS, MemoryGraph, read_all_memories};
use crate::embedding::EmbeddingProvider;
use crate::types::MemoryError;

impl MemoryGraph {
    // -- embedding provider ------------------------------------

    /// Attach a semantic embedding provider for hybrid search.
    ///
    /// This also initialises the vector schema in CozoDB and sets the
    /// hybrid alpha blend factor.  Takes `&self` (not `&mut self`) so it
    /// can be called through `Arc<MemoryGraph>`.
    pub fn set_embedding_provider(
        &self,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Result<(), MemoryError> {
        crate::vector_search::init_embedding_schema(&self.db, provider.dimensions())?;
        let mut guard = self
            .embedding_provider
            .write()
            .map_err(|e| MemoryError::Database(format!("embedding provider lock poisoned: {e}")))?;
        *guard = Some(provider);
        Ok(())
    }

    /// Re-embed every memory in the graph using the currently-configured
    /// embedding provider, overwriting any existing vectors.
    ///
    /// Use after swapping the embedding model (different model variant,
    /// dimension change, or recovery from a corrupted prior model). Each
    /// memory's `content` field is sent through `provider.embed(...)`
    /// and the resulting vector is upserted into `memory_embeddings`.
    /// Memories shorter than `MIN_EMBED_CHARS` are skipped (mirrors the
    /// per-write `embed_and_store` policy).
    ///
    /// Returns `(reindexed, skipped, failed)` counts.
    pub fn reindex_all_embeddings(&self) -> Result<(usize, usize, usize), MemoryError> {
        let provider = {
            let guard = self.embedding_provider.read().map_err(|e| {
                MemoryError::Database(format!("embedding provider lock poisoned: {e}"))
            })?;
            guard
                .clone()
                .ok_or_else(|| MemoryError::Database("no embedding provider configured".into()))?
        };

        let raw_rows = read_all_memories(&self.db)?;
        let mut reindexed = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;

        for raw in raw_rows {
            if raw.content.len() < MIN_EMBED_CHARS {
                skipped += 1;
                continue;
            }
            match provider.embed(std::slice::from_ref(&raw.content)) {
                Ok(vecs) if !vecs.is_empty() => {
                    match crate::vector_search::store_embedding(
                        &self.db,
                        &raw.id,
                        &vecs[0],
                        "auto",
                        provider.dimensions(),
                    ) {
                        Ok(()) => reindexed += 1,
                        Err(e) => {
                            tracing::warn!(memory_id = %raw.id, error = %e, "memory.embedding.store_failed");
                            failed += 1;
                        }
                    }
                }
                Ok(_) => {
                    tracing::warn!(memory_id = %raw.id, "embed returned empty vec");
                    failed += 1;
                }
                Err(e) => {
                    tracing::warn!(memory_id = %raw.id, error = %e, "embed failed");
                    failed += 1;
                }
            }
        }

        Ok((reindexed, skipped, failed))
    }

    /// Set the hybrid alpha for keyword/vector blending.
    pub fn set_hybrid_alpha(&self, alpha: f32) {
        if let Ok(mut guard) = self.hybrid_alpha.write() {
            *guard = alpha.clamp(0.0, 1.0);
        }
    }

    /// Read the current hybrid alpha value.
    pub(super) fn read_hybrid_alpha(&self) -> f32 {
        self.hybrid_alpha.read().map(|g| *g).unwrap_or(0.3)
    }

    /// Embed content and store the vector alongside the memory.
    pub(super) fn embed_and_store(
        &self,
        memory_id: &str,
        content: &str,
    ) -> Result<(), MemoryError> {
        if content.len() < MIN_EMBED_CHARS {
            return Ok(());
        }
        let provider = match self.embedding_provider.read() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                return Err(MemoryError::Database(format!(
                    "embedding provider lock poisoned: {e}"
                )));
            }
        };
        if let Some(ref provider) = provider {
            match provider.embed(&[content.to_string()]) {
                Ok(vecs) if !vecs.is_empty() => crate::vector_search::store_embedding(
                    &self.db,
                    memory_id,
                    &vecs[0],
                    "auto",
                    provider.dimensions(),
                ),
                Ok(_) => Err(MemoryError::Embedding(
                    "embedding provider returned no vectors".into(),
                )),
                Err(e) => Err(e),
            }
        } else {
            Ok(())
        }
    }
}
