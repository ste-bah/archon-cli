use super::*;
use archon_memory::{Memory, MemoryError, MemoryGraph, StoreMemoryOutcome};

enum FailurePoint {
    CorrectionStore,
    ScoreUpdate,
    ScoreUpdateAfterCommitOnce(std::sync::atomic::AtomicBool),
    ScoreUpdateAfterCommit,
    StatusReadError,
    ExplicitRuleLookup,
}

struct FailingMemory<'a> {
    inner: &'a MemoryGraph,
    failure_point: FailurePoint,
}

struct OwnershipRaceMemory {
    inner: std::sync::Arc<MemoryGraph>,
    correction_id: String,
    preflight_barrier: std::sync::Barrier,
    creator_thread: std::sync::Mutex<Option<std::thread::ThreadId>>,
    creator_ready: std::sync::Condvar,
    creator_boosted: std::sync::Condvar,
    creator_finished_boost: std::sync::Mutex<bool>,
    boost_turn: std::sync::atomic::AtomicUsize,
}

impl OwnershipRaceMemory {
    fn new(inner: std::sync::Arc<MemoryGraph>, correction_id: &str) -> Self {
        Self {
            inner,
            correction_id: correction_id.to_string(),
            preflight_barrier: std::sync::Barrier::new(2),
            creator_thread: std::sync::Mutex::new(None),
            creator_ready: std::sync::Condvar::new(),
            creator_boosted: std::sync::Condvar::new(),
            creator_finished_boost: std::sync::Mutex::new(false),
            boost_turn: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

include!("support_sync_impls.rs");

impl MemoryTrait for FailingMemory<'_> {
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
        if matches!(self.failure_point, FailurePoint::CorrectionStore)
            && memory_type == MemoryType::Correction
        {
            return Err(MemoryError::Database(
                "injected correction-store failure".to_string(),
            ));
        }
        self.inner.store_memory(
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
        if matches!(self.failure_point, FailurePoint::CorrectionStore)
            && memory_type == MemoryType::Correction
        {
            return Err(MemoryError::Database(
                "injected correction-store failure".to_string(),
            ));
        }
        self.inner.store_memory_with_id_outcome(
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
        if matches!(self.failure_point, FailurePoint::CorrectionStore)
            && memory_type == MemoryType::Correction
        {
            return Err(MemoryError::Database(
                "injected correction-store failure".to_string(),
            ));
        }
        self.inner.store_memory_with_id(
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
        self.inner.get_memory(id)
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        if matches!(self.failure_point, FailurePoint::ExplicitRuleLookup)
            && id == "rule-lookup-failure"
        {
            return Err(MemoryError::Database(
                "injected explicit-rule lookup failure".to_string(),
            ));
        }
        self.inner.inspect_memory(id)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        self.inner.update_memory(id, content, tags)
    }

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        if matches!(self.failure_point, FailurePoint::ScoreUpdate) {
            return Err(MemoryError::Database(
                "injected score-update failure".to_string(),
            ));
        }
        let updated = self.inner.apply_importance_delta(id, delta, provenance_id);
        if matches!(self.failure_point, FailurePoint::ScoreUpdateAfterCommit)
            || matches!(self.failure_point, FailurePoint::StatusReadError)
        {
            updated.expect("commit delta before simulating lost response");
            return Err(MemoryError::Database(
                "injected lost score-update response after commit".to_string(),
            ));
        }
        if let FailurePoint::ScoreUpdateAfterCommitOnce(failed_once) = &self.failure_point
            && !failed_once.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            updated.expect("commit delta before simulating lost response");
            return Err(MemoryError::Database(
                "injected lost score-update response after commit".to_string(),
            ));
        }
        updated
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        if matches!(self.failure_point, FailurePoint::StatusReadError) {
            return Err(MemoryError::Database(
                "injected provenance-status read failure".to_string(),
            ));
        }
        self.inner
            .has_importance_application(memory_id, provenance_id)
    }

    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> {
        self.inner.delete_memory(id)
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), MemoryError> {
        self.inner
            .create_relationship(from_id, to_id, rel_type, context, strength)
    }

    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        self.inner.recall_memories(query, limit)
    }

    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        self.inner.search_memories(filter)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        self.inner.list_recent(limit)
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        self.inner.memory_count()
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        self.inner.clear_all()
    }

    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        self.inner.get_related_memories(id, depth)
    }
}

fn make_tracker() -> (MemoryGraph, ()) {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should succeed");
    (graph, ())
}

fn memory_rows(graph: &MemoryGraph) -> serde_json::Value {
    let mut memories = [MemoryType::Correction, MemoryType::Rule]
        .into_iter()
        .flat_map(|memory_type| {
            graph
                .search_memories(&SearchFilter {
                    memory_type: Some(memory_type),
                    ..Default::default()
                })
                .expect("snapshot memory rows")
        })
        .collect::<Vec<_>>();
    memories.sort_by(|left, right| left.id.cmp(&right.id));
    serde_json::to_value(memories).expect("serialize memory rows")
}
