// Bounded-query evidence for `recall_corrections`: a recording `MemoryTrait`
// double that captures the `SearchFilter` it is handed, plus the tests that
// assert the filter carries a real bound wider than the caller's limit.
//
// Split from `recall.rs` to keep both files under the 500-line gate. Included
// by `tests.rs` via `include!`, so it shares that module's scope and needs no
// imports of its own.

/// Records every `SearchFilter` handed to `search_memories`, delegating the
/// actual work to a real in-memory graph.
struct FilterRecordingMemory {
    inner: MemoryGraph,
    filters: std::sync::Mutex<Vec<SearchFilter>>,
}

impl FilterRecordingMemory {
    fn new() -> Self {
        Self {
            inner: MemoryGraph::in_memory().expect("in-memory graph"),
            filters: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn recorded(&self) -> Vec<SearchFilter> {
        self.filters.lock().expect("filter log").clone()
    }
}

impl MemoryTrait for FilterRecordingMemory {
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
        self.inner.apply_importance_delta(id, delta, provenance_id)
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, MemoryError> {
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
        self.filters
            .lock()
            .expect("filter log")
            .push(filter.clone());
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

/// The bug: `recall_corrections` issued an unbounded query on every iteration
/// of the agent loop and truncated afterwards. Before the fix the recorded
/// filter carried no limit at all, so `candidate_limit()` was `usize::MAX`.
#[test]
fn recall_corrections_issues_a_bounded_query() {
    let memory = FilterRecordingMemory::new();
    let tracker = CorrectionTracker::new(&memory);

    tracker
        .recall_corrections("some user context", 5)
        .expect("recall");

    let filters = memory.recorded();
    assert_eq!(filters.len(), 1, "one search per recall");
    let limit = filters[0].limit.expect(
        "recall_corrections must bound its query; an unset limit means \
         `read every matching row`, which is the bug",
    );
    assert!(
        limit < usize::MAX,
        "query limit must be a real bound, got {limit}"
    );
    assert!(
        limit > 5,
        "the pool must be wider than the caller's limit or the severity \
         re-rank below it can never see the winner, got {limit}"
    );
}

#[test]
fn recall_corrections_asks_for_nothing_when_limit_is_zero() {
    let memory = FilterRecordingMemory::new();
    let tracker = CorrectionTracker::new(&memory);

    let results = tracker.recall_corrections("some user context", 0).expect("recall");

    assert!(results.is_empty());
    assert_eq!(memory.recorded()[0].limit, Some(0));
}

/// Guard for the interaction the bound could break: the highest-severity
/// correction must survive even when it is buried behind many others, because
/// the severity ranking runs *after* the query.
#[test]
fn recall_corrections_finds_high_severity_behind_many_low_severity_matches() {
    let (graph, _) = make_tracker();
    let tracker = CorrectionTracker::new(&graph);

    let critical = tracker
        .record_correction(
            CorrectionType::ActedWithoutPermission,
            "buried marker correction detail",
            "first",
            None,
        )
        .expect("record critical correction");
    for index in 0..40 {
        tracker
            .record_correction(
                CorrectionType::FactualError,
                &format!("buried marker correction detail decoy {index}"),
                "bulk",
                None,
            )
            .expect("record decoy");
    }

    let results = tracker
        .recall_corrections("buried marker correction detail", 1)
        .expect("recall corrections");

    assert_eq!(
        results[0].id, critical.id,
        "severity ranking must still see the whole candidate window"
    );
}
