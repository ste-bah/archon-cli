use super::*;
use archon_memory::types::{
    Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome,
};
use archon_memory::{MemoryGraph, MemoryTrait};
use std::sync::{Arc, Barrier};

struct BlockingSearchMemory {
    graph: Arc<MemoryGraph>,
    search_completed: Arc<Barrier>,
    resume_import: Arc<Barrier>,
}

impl MemoryTrait for BlockingSearchMemory {
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
        self.graph.store_memory(
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
        self.graph.store_memory_with_id_outcome(
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
        self.graph.store_memory_with_id(
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
        self.graph.get_memory(id)
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        self.graph.inspect_memory(id)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        self.graph.update_memory(id, content, tags)
    }

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        self.graph.apply_importance_delta(id, delta, provenance_id)
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        self.graph
            .has_importance_application(memory_id, provenance_id)
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        self.graph.delete_memory(id)
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        self.graph
            .create_relationship(from_id, to_id, rel_type, context, strength)
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        self.graph.recall_memories(query, limit)
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        let memories = self.graph.search_memories(filter)?;
        self.search_completed.wait();
        self.resume_import.wait();
        Ok(memories)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        self.graph.list_recent(limit)
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        self.graph.memory_count()
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        self.graph.clear_all()
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        self.graph.get_related_memories(id, depth)
    }
}

fn make_engine() -> (MemoryGraph, ()) {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should succeed");
    (graph, ())
}

include!("tests/core.rs");
include!("tests/reinforcement.rs");
include!("tests/scores.rs");
include!("tests/concurrency.rs");
