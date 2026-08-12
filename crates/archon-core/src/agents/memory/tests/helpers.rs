use super::*;
use archon_memory::StoreMemoryOutcome;

/// Null memory implementation for testing. Returns empty results.
pub(crate) struct NullMemory;

impl MemoryTrait for NullMemory {
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
        Ok("null-id".to_string())
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
        Err(MemoryError::NotFound("null".into()))
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
        Err(MemoryError::NotFound("null".into()))
    }
    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        Err(MemoryError::NotFound("null".into()))
    }
    fn inspect_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        Err(MemoryError::NotFound("null".into()))
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
        Err(MemoryError::NotFound("test double".into()))
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
        _from: &str,
        _to: &str,
        _rel: RelType,
        _ctx: Option<&str>,
        _str: f64,
    ) -> Result<(), MemoryError> {
        Ok(())
    }
    fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        Ok(vec![])
    }
    fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        Ok(vec![])
    }
    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        Ok(vec![])
    }
    fn memory_count(&self) -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn clear_all(&self) -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        Ok(vec![])
    }
}

/// Mock memory that records store and search calls for verification.
pub(crate) struct MockMemory {
    pub(crate) stored: Mutex<Vec<(String, Vec<String>)>>, // (content, tags)
    /// Filters passed to `search_memories`, in call order. Recall-cache tests
    /// assert on the length: one entry per query actually run.
    pub(crate) searches: Mutex<Vec<SearchFilter>>,
    /// Rows every `search_memories` call returns.
    pub(crate) search_results: Mutex<Vec<Memory>>,
}

impl MockMemory {
    pub(crate) fn new() -> Self {
        Self {
            stored: Mutex::new(vec![]),
            searches: Mutex::new(vec![]),
            search_results: Mutex::new(vec![]),
        }
    }

    /// Number of `search_memories` calls seen so far.
    pub(crate) fn search_count(&self) -> usize {
        self.searches.lock().unwrap().len()
    }

    /// Replace the canned rows every subsequent search returns.
    pub(crate) fn set_search_results(&self, contents: &[&str]) {
        *self.search_results.lock().unwrap() =
            contents.iter().map(|c| memory_row(c)).collect::<Vec<_>>();
    }
}

/// Build a minimal `Memory` row; only `content` is read by agent recall.
pub(crate) fn memory_row(content: &str) -> Memory {
    Memory {
        id: format!("mock-{content}"),
        content: content.to_string(),
        title: String::new(),
        memory_type: MemoryType::Fact,
        importance: 0.5,
        tags: vec![],
        source_type: "test".to_string(),
        project_path: String::new(),
        created_at: chrono::Utc::now(),
        updated_at: None,
        access_count: 0,
        last_accessed: None,
    }
}

impl MemoryTrait for MockMemory {
    fn store_memory(
        &self,
        content: &str,
        _title: &str,
        _memory_type: MemoryType,
        _importance: f64,
        tags: &[String],
        _source_type: &str,
        _project_path: &str,
    ) -> Result<String, MemoryError> {
        self.stored
            .lock()
            .unwrap()
            .push((content.to_string(), tags.to_vec()));
        Ok("mock-id".to_string())
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
        Err(MemoryError::NotFound("mock".into()))
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
        Err(MemoryError::NotFound("mock".into()))
    }
    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        Err(MemoryError::NotFound("mock".into()))
    }
    fn inspect_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        Err(MemoryError::NotFound("mock".into()))
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
        Err(MemoryError::NotFound("test double".into()))
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
        _from: &str,
        _to: &str,
        _rel: RelType,
        _ctx: Option<&str>,
        _str: f64,
    ) -> Result<(), MemoryError> {
        Ok(())
    }
    fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        Ok(vec![])
    }
    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        self.searches.lock().unwrap().push(filter.clone());
        Ok(self.search_results.lock().unwrap().clone())
    }
    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        Ok(vec![])
    }
    fn memory_count(&self) -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn clear_all(&self) -> Result<usize, MemoryError> {
        Ok(0)
    }
    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        Ok(vec![])
    }
}
