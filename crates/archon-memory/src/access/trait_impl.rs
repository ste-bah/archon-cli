use std::sync::Arc;

use crate::graph::MemoryGraph;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome};

use super::{MemoryAccess, MemoryTrait};

// ── MemoryGraph impl ───────────────────────────────────────────

impl MemoryTrait for MemoryGraph {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        MemoryGraph::store_memory(
            self,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn store_memory_with_id_outcome(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError> {
        MemoryGraph::store_memory_with_id_outcome(
            self,
            id,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn store_memory_with_id(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<Memory, MemoryError> {
        MemoryGraph::store_memory_with_id(
            self,
            id,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        MemoryGraph::get_memory(self, id)
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        self.read_memory(id)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        MemoryGraph::update_memory(self, id, content, tags)
    }

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        MemoryGraph::apply_importance_delta(self, id, delta, provenance_id)
    }

    fn reconcile_importance_trend(
        &self,
        id: &str,
        previous_importance: f64,
    ) -> Result<Memory, MemoryError> {
        MemoryGraph::reconcile_importance_trend(self, id, previous_importance)
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        MemoryGraph::has_importance_application(self, memory_id, provenance_id)
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        MemoryGraph::delete_memory(self, id)
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        MemoryGraph::create_relationship(self, from_id, to_id, rel_type, context, strength)
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::recall_memories(self, query, limit)
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::search_memories(self, filter)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::list_recent(self, limit)
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        MemoryGraph::memory_count(self)
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        MemoryGraph::clear_all(self)
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        MemoryGraph::get_related_memories(self, id, depth)
    }

    fn embedding_neighbours(
        &self,
        memory_id: &str,
        top_k: usize,
    ) -> Result<Vec<(String, f64)>, MemoryError> {
        // A graph that never initialised the embedding schema has no
        // `memory_embeddings` relation at all, and querying it errors. That is
        // the same "no vector search here" condition as a missing row, so it
        // degrades rather than propagating -- otherwise one unindexed store
        // fails the entire consolidation pass.
        let vector = match crate::vector_search::fetch_embedding(self.db(), memory_id) {
            Ok(Some(vector)) => vector,
            Ok(None) => return Ok(Vec::new()),
            Err(error) => {
                tracing::debug!(%error, "no embedding index; skipping neighbour search");
                return Ok(Vec::new());
            }
        };
        // `top_k + 1` because the query vector is this memory's own, so it
        // always returns itself as the nearest hit.
        let hits = match crate::vector_search::search_similar(self.db(), &vector, top_k + 1) {
            Ok(hits) => hits,
            Err(error) => {
                // No HNSW index, or one built for a different dimension. The
                // contract is that empty means "unavailable", so the caller
                // falls back rather than the whole pass failing.
                tracing::debug!(%error, "embedding neighbour search unavailable");
                return Ok(Vec::new());
            }
        };
        Ok(hits
            .into_iter()
            .filter(|(id, _)| id != memory_id)
            .take(top_k)
            .collect())
    }
}

// ── MemoryAccess impl ───────────────────────────────────────────

impl MemoryAccess {
    /// Return the underlying [`MemoryGraph`] if this is a `Direct` access,
    /// or `None` for a `Remote` client.
    pub fn graph(&self) -> Option<&Arc<MemoryGraph>> {
        match self {
            Self::Direct { graph, .. } => Some(graph),
            Self::Remote(_) => None,
        }
    }
}

impl MemoryTrait for MemoryAccess {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.store_memory(
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
            Self::Remote(client) => client.store_memory(
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
        }
    }

    fn store_memory_with_id_outcome(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.store_memory_with_id_outcome(
                id,
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
            Self::Remote(client) => client.store_memory_with_id_outcome(
                id,
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
        }
    }

    fn store_memory_with_id(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<Memory, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.store_memory_with_id(
                id,
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
            Self::Remote(client) => client.store_memory_with_id(
                id,
                content,
                title,
                memory_type,
                importance,
                tags,
                source_type,
                project_path,
            ),
        }
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.get_memory(id),
            Self::Remote(client) => client.get_memory(id),
        }
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.read_memory(id),
            Self::Remote(client) => client.inspect_memory(id),
        }
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.update_memory(id, content, tags),
            Self::Remote(client) => client.update_memory(id, content, tags),
        }
    }

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.apply_importance_delta(id, delta, provenance_id),
            Self::Remote(client) => client.apply_importance_delta(id, delta, provenance_id),
        }
    }

    fn reconcile_importance_trend(
        &self,
        id: &str,
        previous_importance: f64,
    ) -> Result<Memory, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.reconcile_importance_trend(id, previous_importance),
            Self::Remote(client) => client.reconcile_importance_trend(id, previous_importance),
        }
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        match self {
            Self::Direct { graph, .. } => {
                graph.has_importance_application(memory_id, provenance_id)
            }
            Self::Remote(client) => client.has_importance_application(memory_id, provenance_id),
        }
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.delete_memory(id),
            Self::Remote(client) => client.delete_memory(id),
        }
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        match self {
            Self::Direct { graph, .. } => {
                graph.create_relationship(from_id, to_id, rel_type, context, strength)
            }
            Self::Remote(client) => {
                client.create_relationship(from_id, to_id, rel_type, context, strength)
            }
        }
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.recall_memories(query, limit),
            Self::Remote(client) => client.recall_memories(query, limit),
        }
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.search_memories(filter),
            Self::Remote(client) => client.search_memories(filter),
        }
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.list_recent(limit),
            Self::Remote(client) => client.list_recent(limit),
        }
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.memory_count(),
            Self::Remote(client) => client.memory_count(),
        }
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.clear_all(),
            Self::Remote(client) => client.clear_all(),
        }
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.get_related_memories(id, depth),
            Self::Remote(client) => client.get_related_memories(id, depth),
        }
    }

    fn embedding_neighbours(
        &self,
        memory_id: &str,
        top_k: usize,
    ) -> Result<Vec<(String, f64)>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.embedding_neighbours(memory_id, top_k),
            // The memory-server protocol has no vector-neighbour request, so a
            // remote store falls back to the lexical pass. Adding one is a
            // protocol change and belongs with its own round-trip tests.
            Self::Remote(_) => Ok(Vec::new()),
        }
    }
}
