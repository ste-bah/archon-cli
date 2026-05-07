use super::*;

/// Null memory implementation for testing. Returns empty results.
pub(super) struct NullMemory;

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
    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
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
    fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> {
        Ok(())
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

/// Mock memory that records store calls for verification.
pub(super) struct MockMemory {
    pub(super) stored: Mutex<Vec<(String, Vec<String>)>>, // (content, tags)
}

impl MockMemory {
    pub(super) fn new() -> Self {
        Self {
            stored: Mutex::new(vec![]),
        }
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
    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
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
    fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> {
        Ok(())
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
