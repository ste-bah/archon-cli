use std::sync::{Arc, RwLock};

use cozo::DbInstance;

use crate::embedding::EmbeddingProvider;

mod constructors;
mod crud;
mod embeddings;
mod helpers;
mod queries;
mod relationships;
mod rows;
mod schema;
#[cfg(test)]
mod tests;
mod traversal;

pub(crate) use rows::{raw_to_memory, read_all_memories, row_to_memory};

/// Minimum content length required to generate an embedding.
pub(super) const MIN_EMBED_CHARS: usize = 10;

/// CozoDB-backed memory graph.
pub struct MemoryGraph {
    pub(crate) db: DbInstance,
    embedding_provider: RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    hybrid_alpha: RwLock<f32>,
}
