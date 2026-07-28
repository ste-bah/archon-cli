use super::*;
use std::sync::{Arc, Mutex};

use archon_memory::types::{
    Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome,
};
use chrono::{TimeZone, Utc};

use crate::command::dispatcher::Dispatcher;
use crate::command::registry::{CommandContext, RegistryBuilder};

/// Inline TestMemory double used by the B18 tests.
///
/// Mirrors the AGS-817 /memory `TestMemory` pattern (scoped
/// locally rather than extending `archon_test_support::memory::
/// MockMemoryTrait` so the B18 blast radius stays in this file).
/// Only `recall_memories` is exercised by `RecallHandler`; every
/// other trait method panics with `unimplemented!()`.
struct StubMemory {
    recall_result: Mutex<Result<Vec<Memory>, MemoryError>>,
    recall_captured_query: Mutex<Option<String>>,
}

impl StubMemory {
    fn new(result: Result<Vec<Memory>, MemoryError>) -> Self {
        Self {
            recall_result: Mutex::new(result),
            recall_captured_query: Mutex::new(None),
        }
    }

    fn captured_query(&self) -> Option<String> {
        self.recall_captured_query.lock().unwrap().clone()
    }
}

/// Clone a `Result<Vec<Memory>, MemoryError>` by round-tripping
/// the error variant through Display (MemoryError doesn't derive
/// Clone). Mirrors the AGS-817 /memory `clone_result` helper.
fn clone_result(r: &Result<Vec<Memory>, MemoryError>) -> Result<Vec<Memory>, MemoryError> {
    match r {
        Ok(v) => Ok(v.clone()),
        Err(e) => Err(MemoryError::Database(format!("{e}"))),
    }
}

impl MemoryTrait for StubMemory {
    fn store_memory(
        &self,
        _content: &str,
        _title: &str,
        _memory_type: MemoryType,
        _importance: f64,
        _tags: &[String],
        _source_type: &str,
        _project_path: &str,
    ) -> Result<String, MemoryError> {
        unimplemented!("StubMemory: store_memory not used by B18 tests")
    }

    fn store_memory_with_id_outcome(
        &self,
        _id: &str,
        _content: &str,
        _title: &str,
        _memory_type: MemoryType,
        _importance: f64,
        _tags: &[String],
        _source_type: &str,
        _project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError> {
        unimplemented!("StubMemory: store_memory_with_id_outcome not used by tests")
    }

    fn store_memory_with_id(
        &self,
        _id: &str,
        _content: &str,
        _title: &str,
        _memory_type: MemoryType,
        _importance: f64,
        _tags: &[String],
        _source_type: &str,
        _project_path: &str,
    ) -> Result<Memory, MemoryError> {
        unimplemented!("StubMemory: store_memory_with_id not used by B18 tests")
    }

    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        unimplemented!("StubMemory: get_memory not used by B18 tests")
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        Err(MemoryError::NotFound(id.to_string()))
    }

    fn update_memory(
        &self,
        _id: &str,
        _content: Option<&str>,
        _tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        unimplemented!("StubMemory: update_memory not used by B18 tests")
    }

    fn apply_importance_delta(
        &self,
        _id: &str,
        _delta: f64,
        _provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        unimplemented!("StubMemory: apply_importance_delta not used by B18 tests")
    }

    fn has_importance_application(
        &self,
        _memory_id: &str,
        _provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        unimplemented!("test double: has_importance_application not used")
    }

    fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
        unimplemented!("StubMemory: delete_memory not used by B18 tests")
    }

    fn create_relationship(
        &self,
        _from_id: &str,
        _to_id: &str,
        _rel_type: RelType,
        _context: Option<&str>,
        _strength: f64,
    ) -> Result<(), MemoryError> {
        unimplemented!("StubMemory: create_relationship not used by B18 tests")
    }

    fn recall_memories(&self, query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        *self.recall_captured_query.lock().unwrap() = Some(query.to_string());
        clone_result(&self.recall_result.lock().unwrap())
    }

    fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("StubMemory: search_memories not used by B18 tests")
    }

    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("StubMemory: list_recent not used by B18 tests")
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        unimplemented!("StubMemory: memory_count not used by B18 tests")
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        unimplemented!("StubMemory: clear_all not used by B18 tests")
    }

    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("StubMemory: get_related_memories not used by B18 tests")
    }
}

/// Build a `Memory` record for use in match-path test fixtures.
fn make_mem(id: &str, title: &str, content: &str) -> Memory {
    Memory {
        id: id.to_string(),
        content: content.to_string(),
        title: title.to_string(),
        memory_type: MemoryType::Fact,
        importance: 0.5,
        tags: Vec::new(),
        source_type: "test".to_string(),
        project_path: "/tmp/test".to_string(),
        created_at: Utc.with_ymd_and_hms(2026, 4, 20, 12, 0, 0).unwrap(),
        updated_at: None,
        access_count: 0,
        last_accessed: None,
    }
}

/// Build a `CommandContext` with a freshly-created channel and the
/// supplied `memory` handle. Mirrors the AGS-817 /memory
/// `make_ctx(memory)` fixture — DIRECT pattern, no snapshot, no
/// effect slot.
fn make_recall_ctx(
    memory: Option<Arc<dyn MemoryTrait>>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_memory_opt(memory)
        .build()
}

/// R4: description is byte-identical to the `declare_handler!`
/// stub at registry.rs:1305. Any drift here means the stub and
/// the new handler have diverged — Sherlock will flag it.
mod cases_a;
mod cases_b;
