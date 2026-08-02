use std::sync::{Arc, RwLock};

use crate::embedding::EmbeddingProvider;

mod constructors;
mod crud;
mod embeddings;
pub(crate) mod helpers;
mod queries;
mod relationships;
mod rows;
mod schema;
#[cfg(test)]
mod tests;
mod traversal;

pub(crate) use rows::{
    raw_to_memory, read_all_memories, row_to_memory, row_values_to_memory, rows_to_memories,
};

/// Minimum content length required to generate an embedding.
pub(super) const MIN_EMBED_CHARS: usize = 10;

/// CozoDB-backed memory graph.
pub struct MemoryGraph {
    /// Guarded handle over the Cozo store.
    ///
    /// Registering the instance is what lets `run_bound_script_guarded` resolve
    /// the write-lock config by pointer identity from the free functions that
    /// only receive a `&DbInstance` (`vector_search`, `search`, `garden`).
    /// `Deref` keeps `&self.db` usable wherever a `&DbInstance` is expected.
    pub(crate) db: archon_cozo::GuardedDbInstance,
    embedding_provider: RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    hybrid_alpha: RwLock<f32>,
}
