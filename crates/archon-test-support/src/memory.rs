//! `MockMemoryTrait` — test double for `archon_memory::MemoryTrait`
//! (REQ-FOR-PRESERVE-D8).
//!
//! This implements the REAL 12-method `archon_memory::access::MemoryTrait`
//! so callers can pass `&MockMemoryTrait` anywhere a `&dyn MemoryTrait` is
//! expected. `store_memory` records every call into a shared call log;
//! every other method panics with `unimplemented!()` so any refactor that
//! starts touching a different API is caught loudly.
//!
//! Prior to 2026-04-11 this module defined a local 1-method `MemoryTraitLike`
//! to keep archon-memory out of the dev-dep graph. Forensic audit of
//! TASK-AGS-008 flagged that as a design miss (the mock couldn't actually
//! be used where its consumers needed it); archon-memory was already a
//! production dep of every crate that pulls in archon-test-support at dev
//! time, so there is no leak concern from taking the dep here.

use std::sync::{Arc, Mutex};

use archon_memory::MemoryTrait;
use archon_memory::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};

/// A single `store_memory` call recorded by [`MockMemoryTrait`]. Captures
/// all seven arguments so regression guards can assert the full invariant.
#[derive(Debug, Clone)]
pub struct StoredCall {
    pub content: String,
    pub title: String,
    pub memory_type: MemoryType,
    pub importance: f64,
    pub tags: Vec<String>,
    pub source_type: String,
    pub project_path: String,
}

/// Recording double. Cheap to clone; the call log is shared via `Arc`.
#[derive(Debug, Clone, Default)]
pub struct MockMemoryTrait {
    calls: Arc<Mutex<Vec<StoredCall>>>,
}

impl MockMemoryTrait {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every recorded `store_memory` call in insertion order.
    pub fn calls(&self) -> Vec<StoredCall> {
        self.calls
            .lock()
            .expect("MockMemoryTrait call log poisoned")
            .clone()
    }
}

impl MemoryTrait for MockMemoryTrait {
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
        self.calls
            .lock()
            .expect("MockMemoryTrait call log poisoned")
            .push(StoredCall {
                content: content.to_string(),
                title: title.to_string(),
                memory_type,
                importance,
                tags: tags.to_vec(),
                source_type: source_type.to_string(),
                project_path: project_path.to_string(),
            });
        Ok("mock-memory-id".to_string())
    }

    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        unimplemented!("MockMemoryTrait: get_memory not used by save_agent_memory")
    }

    fn update_memory(
        &self,
        _id: &str,
        _content: Option<&str>,
        _tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        unimplemented!("MockMemoryTrait: update_memory not used by save_agent_memory")
    }

    fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> {
        unimplemented!("MockMemoryTrait: update_importance not used by save_agent_memory")
    }

    fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
        unimplemented!("MockMemoryTrait: delete_memory not used by save_agent_memory")
    }

    fn create_relationship(
        &self,
        _from_id: &str,
        _to_id: &str,
        _rel_type: RelType,
        _context: Option<&str>,
        _strength: f64,
    ) -> Result<(), MemoryError> {
        unimplemented!("MockMemoryTrait: create_relationship not used by save_agent_memory")
    }

    fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("MockMemoryTrait: recall_memories not used by save_agent_memory")
    }

    fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("MockMemoryTrait: search_memories not used by save_agent_memory")
    }

    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("MockMemoryTrait: list_recent not used by save_agent_memory")
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        unimplemented!("MockMemoryTrait: memory_count not used by save_agent_memory")
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        unimplemented!("MockMemoryTrait: clear_all not used by save_agent_memory")
    }

    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("MockMemoryTrait: get_related_memories not used by save_agent_memory")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_store_memory_calls_in_order() {
        let m = MockMemoryTrait::new();
        let _ = m.store_memory(
            "alpha",
            "title-a",
            MemoryType::Fact,
            0.5,
            &["t1".into()],
            "agent",
            "/p",
        );
        let _ = m.store_memory(
            "beta",
            "title-b",
            MemoryType::Fact,
            0.7,
            &["t2".into(), "t3".into()],
            "agent",
            "/p",
        );
        let calls = m.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].content, "alpha");
        assert_eq!(calls[0].title, "title-a");
        assert_eq!(calls[1].tags, vec!["t2".to_string(), "t3".to_string()]);
    }

    #[test]
    fn usable_as_dyn_memory_trait() {
        let m: &dyn MemoryTrait = &MockMemoryTrait::new();
        let id = m
            .store_memory("c", "t", MemoryType::Fact, 0.5, &[], "agent", "/p")
            .expect("store_memory via dyn");
        assert_eq!(id, "mock-memory-id");
    }
}
