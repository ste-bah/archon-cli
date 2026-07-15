use super::*;
use archon_memory::types::{
    Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome,
};
use chrono::{TimeZone, Utc};
use std::sync::Arc;
use std::sync::Mutex;

/// Inline TestMemory double used by the AGS-817 tests.
///
/// `archon_test_support::memory::MockMemoryTrait` exists but every
/// non-store method panics with `unimplemented!()`. The AGS-817
/// tests exercise `list_recent`, `recall_memories`, and `clear_all`
/// — so we define a local double that returns configurable
/// pre-canned values. Defined here rather than extending the shared
/// mock so the AGS-817 blast radius stays scoped to this file.
struct TestMemory {
    list_recent_result: Mutex<Result<Vec<Memory>, MemoryError>>,
    recall_result: Mutex<Result<Vec<Memory>, MemoryError>>,
    clear_result: Mutex<Result<usize, MemoryError>>,
    recall_captured_query: Mutex<Option<String>>,
}

impl TestMemory {
    fn new() -> Self {
        Self {
            list_recent_result: Mutex::new(Ok(Vec::new())),
            recall_result: Mutex::new(Ok(Vec::new())),
            clear_result: Mutex::new(Ok(0)),
            recall_captured_query: Mutex::new(None),
        }
    }

    fn with_list_recent(self, r: Result<Vec<Memory>, MemoryError>) -> Self {
        *self.list_recent_result.lock().unwrap() = r;
        self
    }

    fn with_recall(self, r: Result<Vec<Memory>, MemoryError>) -> Self {
        *self.recall_result.lock().unwrap() = r;
        self
    }

    fn with_clear(self, r: Result<usize, MemoryError>) -> Self {
        *self.clear_result.lock().unwrap() = r;
        self
    }

    fn captured_recall_query(&self) -> Option<String> {
        self.recall_captured_query.lock().unwrap().clone()
    }
}

// Clone `MemoryError` by round-tripping via Display — MemoryError
// doesn't derive Clone in the shipped types, but our test doubles
// need to return the same Err on repeat calls. We box the Display
// form into the Database variant (arbitrarily chosen since AGS-817
// tests don't exercise the error-variant-distinguishing path). Only
// used internally to this test module.
fn clone_result<T: Clone>(r: &Result<T, MemoryError>) -> Result<T, MemoryError> {
    match r {
        Ok(v) => Ok(v.clone()),
        Err(e) => Err(MemoryError::Database(format!("{e}"))),
    }
}

impl MemoryTrait for TestMemory {
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
        unimplemented!("TestMemory: store_memory not used by AGS-817 tests")
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
        unimplemented!("TestMemory: store_memory_with_id_outcome not used by tests")
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
        unimplemented!("TestMemory: store_memory_with_id not used by AGS-817 tests")
    }

    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        unimplemented!("TestMemory: get_memory not used by AGS-817 tests")
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
        unimplemented!("TestMemory: update_memory not used by AGS-817 tests")
    }

    fn apply_importance_delta(
        &self,
        _id: &str,
        _delta: f64,
        _provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        unimplemented!("TestMemory: apply_importance_delta not used by AGS-817 tests")
    }

    fn has_importance_application(
        &self,
        _memory_id: &str,
        _provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        unimplemented!("test double: has_importance_application not used")
    }

    fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
        unimplemented!("TestMemory: delete_memory not used by AGS-817 tests")
    }

    fn create_relationship(
        &self,
        _from_id: &str,
        _to_id: &str,
        _rel_type: RelType,
        _context: Option<&str>,
        _strength: f64,
    ) -> Result<(), MemoryError> {
        unimplemented!("TestMemory: create_relationship not used by AGS-817 tests")
    }

    fn recall_memories(&self, query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        *self.recall_captured_query.lock().unwrap() = Some(query.to_string());
        clone_result(&self.recall_result.lock().unwrap())
    }

    fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("TestMemory: search_memories not used by AGS-817 tests")
    }

    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        clone_result(&self.list_recent_result.lock().unwrap())
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        unimplemented!("TestMemory: memory_count not used by AGS-817 tests")
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        clone_result(&self.clear_result.lock().unwrap())
    }

    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("TestMemory: get_related_memories not used by AGS-817 tests")
    }
}

/// Build a `Memory` record for use in list / search test fixtures.
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
        created_at: Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap(),
        updated_at: None,
        access_count: 0,
        last_accessed: None,
    }
}

/// Build a `CommandContext` backed by a fresh mpsc channel and the
/// supplied `memory` handle. Mirrors the `make_ctx` fixtures in
/// fork.rs / voice.rs / hooks.rs.
///
/// Every optional field other than `memory` stays `None` — `/memory`
/// is a DIRECT-pattern handler and does not consume any of the
/// typed snapshots.
fn make_ctx(
    memory: Option<Arc<dyn MemoryTrait>>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_memory_opt(memory)
        .build()
}

mod cases_a;
mod cases_b;
