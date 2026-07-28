use super::*;
use archon_memory::garden::GardenConfig;
use archon_memory::types::{
    Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome,
};
use std::sync::Mutex;

/// Inline TestMemory double used by the B13 tests.
///
/// `archon_test_support::memory::MockMemoryTrait` exists but every
/// non-store method panics with `unimplemented!()`. B13 needs a
/// memory double that (a) answers `format_garden_stats(memory, 10)`
/// deterministically and (b) lets `consolidate(memory, &config)`
/// complete end-to-end without panicking. The consolidate path
/// calls `memory_count`, `list_recent`, `search_memories`,
/// `store_memory`, and optionally the update/decay/delete methods;
/// on a fully-empty graph we only need the first four to return
/// `Ok(empty)` / `Ok(0)` and the remainder are never reached. Each
/// method has a configurable result slot so error-path tests can
/// force a deterministic `MemoryError`.
///
/// Defined here rather than extending the shared mock so the B13
/// blast radius stays scoped to this file (matches AGS-817
/// /memory TestMemory precedent).
struct TestMemory {
    count_result: Mutex<Result<usize, MemoryError>>,
    list_recent_result: Mutex<Result<Vec<Memory>, MemoryError>>,
    search_result: Mutex<Result<Vec<Memory>, MemoryError>>,
    store_result: Mutex<Result<String, MemoryError>>,
}

impl TestMemory {
    fn new_empty() -> Self {
        Self {
            count_result: Mutex::new(Ok(0)),
            list_recent_result: Mutex::new(Ok(Vec::new())),
            search_result: Mutex::new(Ok(Vec::new())),
            store_result: Mutex::new(Ok("stored-id".to_string())),
        }
    }

    /// Force every observable entry point to return the same error
    /// (Database variant). `format_garden_stats` calls
    /// `memory_count` first, so driving that slot to Err is
    /// sufficient to exercise the stats-error path. Consolidate
    /// also calls `memory_count` first for `total_before`, so the
    /// same slot covers both paths.
    fn with_count_error(self, msg: &str) -> Self {
        *self.count_result.lock().unwrap() = Err(MemoryError::Database(msg.to_string()));
        self
    }
}

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
        clone_result(&self.store_result.lock().unwrap())
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
        unimplemented!("TestMemory: store_memory_with_id not used by B13 tests")
    }

    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        unimplemented!("TestMemory: get_memory not used by B13 tests")
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
        Ok(())
    }

    fn apply_importance_delta(
        &self,
        _id: &str,
        _delta: f64,
        _provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        unimplemented!("TestMemory: apply_importance_delta not used by B13 tests")
    }

    fn has_importance_application(
        &self,
        _memory_id: &str,
        _provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        unimplemented!("test double: has_importance_application not used")
    }

    fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
        Ok(())
    }

    fn create_relationship(
        &self,
        _from_id: &str,
        _to_id: &str,
        _rel_type: RelType,
        _context: Option<&str>,
        _strength: f64,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        Ok(Vec::new())
    }

    fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        clone_result(&self.search_result.lock().unwrap())
    }

    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        clone_result(&self.list_recent_result.lock().unwrap())
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        match &*self.count_result.lock().unwrap() {
            Ok(n) => Ok(*n),
            // Unwrap the inner message instead of re-wrapping via
            // `format!("{e}")` — the thiserror Display impl on
            // MemoryError::Database prefixes "database error: ",
            // so naively round-tripping through Display and
            // re-wrapping would double-prefix and break the
            // byte-identity assertions.
            Err(MemoryError::Database(msg)) => Err(MemoryError::Database(msg.clone())),
            Err(other) => Err(MemoryError::Database(format!("{other}"))),
        }
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        Ok(0)
    }

    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        Ok(Vec::new())
    }
}

/// Build a `CommandContext` backed by a fresh mpsc channel and the
/// supplied `memory` / `garden_config` handles. Mirrors the
/// `make_ctx` fixtures in memory.rs / fork.rs. Every optional field
/// other than `memory` / `garden_config` stays `None` — /garden is
/// a DIRECT-pattern handler and does not consume any of the typed
/// snapshots.
fn make_ctx(
    memory: Option<Arc<dyn MemoryTrait>>,
    garden_config: Option<GardenConfig>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_memory_opt(memory)
        .with_garden_config_opt(garden_config)
        .build()
}

// ---------------------------------------------------------------
// R1: description + aliases byte-identity tests
// ---------------------------------------------------------------

mod cases_a;
mod cases_b;
