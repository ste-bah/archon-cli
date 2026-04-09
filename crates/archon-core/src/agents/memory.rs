//! Agent-scoped memory operations using CozoDB tag-based isolation.
//!
//! Agent memories are scoped via the tag convention `agent:{agent_type}`.
//! This avoids namespace collisions between different agents' memories.

use archon_memory::types::{MemoryError, MemoryType, SearchFilter};
use archon_memory::MemoryTrait;

use super::definition::AgentMemoryScope;

/// Build the scoping tag for an agent type.
pub fn agent_tag(agent_type: &str) -> String {
    format!("agent:{agent_type}")
}

/// Build the scope isolation tag from an `AgentMemoryScope`.
pub fn scope_tag(scope: &AgentMemoryScope) -> String {
    match scope {
        AgentMemoryScope::User => "scope:user".to_string(),
        AgentMemoryScope::Project => "scope:project".to_string(),
        AgentMemoryScope::Local => "scope:local".to_string(),
    }
}

/// Load agent-scoped memories from the memory store.
///
/// For each query in `recall_queries`, searches memories tagged with
/// `agent:{agent_type}` (and optionally a scope tag) and returns the
/// matching content strings.
///
/// If `memory_scope` is `None`, returns empty — AC-101: no persistent memory.
pub fn load_agent_memory(
    agent_type: &str,
    recall_queries: &[String],
    memory: &dyn MemoryTrait,
    memory_scope: Option<&AgentMemoryScope>,
) -> Vec<String> {
    // AC-101: None = no persistent memory
    let scope = match memory_scope {
        Some(s) => s,
        None => return Vec::new(),
    };

    let agent = agent_tag(agent_type);
    let scope = scope_tag(scope);
    let mut results = Vec::new();

    for query in recall_queries {
        let filter = SearchFilter {
            memory_type: None,
            tags: vec![agent.clone(), scope.clone()],
            require_all_tags: true,
            text: Some(query.clone()),
            date_from: None,
            date_to: None,
        };

        match memory.search_memories(&filter) {
            Ok(memories) => {
                for m in memories {
                    results.push(m.content.clone());
                }
            }
            Err(e) => {
                tracing::warn!(agent = agent_type, query = query.as_str(), error = %e,
                    "failed to recall agent memory");
            }
        }
    }

    results
}

/// Save a memory scoped to a specific agent.
///
/// Tags the memory with `agent:{agent_type}` and `scope:{scope}` plus any
/// additional user tags.
///
/// If `memory_scope` is `None`, this is a no-op (AC-101: no persistent memory).
pub fn save_agent_memory(
    agent_type: &str,
    content: &str,
    title: &str,
    extra_tags: &[String],
    memory: &dyn MemoryTrait,
    project_path: &str,
    memory_scope: Option<&AgentMemoryScope>,
) -> Result<String, MemoryError> {
    // AC-101: None = no persistent memory
    let scope = match memory_scope {
        Some(s) => s,
        None => return Ok("skipped-no-scope".to_string()),
    };

    let mut tags = vec![agent_tag(agent_type), scope_tag(scope)];
    tags.extend(extra_tags.iter().cloned());

    memory.store_memory(
        content,
        title,
        MemoryType::Fact,
        0.5,
        &tags,
        "agent",
        project_path,
    )
}

/// Increment the invocation_count in an agent's meta.json.
///
/// Returns the new count, or an error message.
pub fn increment_invocation_count(
    agent_dir: &std::path::Path,
) -> Result<u64, String> {
    let meta_path = agent_dir.join("meta.json");
    let content = std::fs::read_to_string(&meta_path)
        .map_err(|e| format!("failed to read meta.json: {e}"))?;

    let mut meta: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("failed to parse meta.json: {e}"))?;

    let count = meta
        .get("invocation_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;

    meta["invocation_count"] = serde_json::json!(count);
    meta["updated_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());

    let serialized = serde_json::to_string_pretty(&meta)
        .map_err(|e| format!("failed to serialize meta.json: {e}"))?;

    std::fs::write(&meta_path, serialized)
        .map_err(|e| format!("failed to write meta.json: {e}"))?;

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::types::{Memory, MemoryType, RelType};
    use std::sync::Mutex;

    /// Null memory implementation for testing. Returns empty results.
    struct NullMemory;

    impl MemoryTrait for NullMemory {
        fn store_memory(
            &self, _content: &str, _title: &str, _memory_type: MemoryType,
            _importance: f64, _tags: &[String], _source_type: &str, _project_path: &str,
        ) -> Result<String, MemoryError> {
            Ok("null-id".to_string())
        }
        fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
            Err(MemoryError::NotFound("null".into()))
        }
        fn update_memory(&self, _id: &str, _content: Option<&str>, _tags: Option<&[String]>) -> Result<(), MemoryError> { Ok(()) }
        fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> { Ok(()) }
        fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        fn create_relationship(&self, _from: &str, _to: &str, _rel: RelType, _ctx: Option<&str>, _str: f64) -> Result<(), MemoryError> { Ok(()) }
        fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
        fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
        fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
        fn memory_count(&self) -> Result<usize, MemoryError> { Ok(0) }
        fn clear_all(&self) -> Result<usize, MemoryError> { Ok(0) }
        fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
    }

    /// Mock memory that records store calls for verification.
    struct MockMemory {
        stored: Mutex<Vec<(String, Vec<String>)>>, // (content, tags)
    }

    impl MockMemory {
        fn new() -> Self {
            Self { stored: Mutex::new(vec![]) }
        }
    }

    impl MemoryTrait for MockMemory {
        fn store_memory(
            &self, content: &str, _title: &str, _memory_type: MemoryType,
            _importance: f64, tags: &[String], _source_type: &str, _project_path: &str,
        ) -> Result<String, MemoryError> {
            self.stored.lock().unwrap().push((content.to_string(), tags.to_vec()));
            Ok("mock-id".to_string())
        }
        fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
            Err(MemoryError::NotFound("mock".into()))
        }
        fn update_memory(&self, _id: &str, _content: Option<&str>, _tags: Option<&[String]>) -> Result<(), MemoryError> { Ok(()) }
        fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> { Ok(()) }
        fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> { Ok(()) }
        fn create_relationship(&self, _from: &str, _to: &str, _rel: RelType, _ctx: Option<&str>, _str: f64) -> Result<(), MemoryError> { Ok(()) }
        fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
        fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
        fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
        fn memory_count(&self) -> Result<usize, MemoryError> { Ok(0) }
        fn clear_all(&self) -> Result<usize, MemoryError> { Ok(0) }
        fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> { Ok(vec![]) }
    }

    #[test]
    fn agent_tag_format() {
        assert_eq!(agent_tag("code-reviewer"), "agent:code-reviewer");
        assert_eq!(agent_tag("explore"), "agent:explore");
    }

    #[test]
    fn load_agent_memory_empty_queries_returns_empty() {
        let memory = NullMemory;
        let results = load_agent_memory("test-agent", &[], &memory, Some(&AgentMemoryScope::User));
        assert!(results.is_empty());
    }

    #[test]
    fn load_agent_memory_null_memory_returns_empty() {
        let memory = NullMemory;
        let queries = vec!["test query".to_string()];
        let results = load_agent_memory("test-agent", &queries, &memory, Some(&AgentMemoryScope::User));
        assert!(results.is_empty());
    }

    #[test]
    fn load_agent_memory_none_scope_returns_empty() {
        let memory = NullMemory;
        let queries = vec!["test query".to_string()];
        let results = load_agent_memory("test-agent", &queries, &memory, None);
        assert!(results.is_empty(), "AC-101: None scope = no persistent memory");
    }

    #[test]
    fn save_agent_memory_includes_agent_and_scope_tags() {
        let memory = MockMemory::new();
        let result = save_agent_memory(
            "code-reviewer",
            "test content",
            "test title",
            &["extra-tag".to_string()],
            &memory,
            "/tmp",
            Some(&AgentMemoryScope::Project),
        );
        assert!(result.is_ok());

        let stored = memory.stored.lock().unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0].1.contains(&"agent:code-reviewer".to_string()));
        assert!(stored[0].1.contains(&"scope:project".to_string()));
        assert!(stored[0].1.contains(&"extra-tag".to_string()));
    }

    #[test]
    fn save_agent_memory_none_scope_is_noop() {
        let memory = MockMemory::new();
        let result = save_agent_memory(
            "code-reviewer",
            "test content",
            "test title",
            &[],
            &memory,
            "/tmp",
            None,
        );
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "skipped-no-scope");

        let stored = memory.stored.lock().unwrap();
        assert!(stored.is_empty(), "AC-101: None scope = no memory stored");
    }

    #[test]
    fn scope_tag_formats() {
        assert_eq!(scope_tag(&AgentMemoryScope::User), "scope:user");
        assert_eq!(scope_tag(&AgentMemoryScope::Project), "scope:project");
        assert_eq!(scope_tag(&AgentMemoryScope::Local), "scope:local");
    }

    #[test]
    fn increment_invocation_count_on_valid_meta() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.json");
        std::fs::write(
            &meta_path,
            r#"{"version":"1.0","invocation_count":5,"updated_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();

        let count = increment_invocation_count(dir.path()).unwrap();
        assert_eq!(count, 6);

        // Verify the file was updated
        let content = std::fs::read_to_string(&meta_path).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(meta["invocation_count"], 6);
    }

    #[test]
    fn increment_invocation_count_missing_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let result = increment_invocation_count(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn increment_invocation_count_missing_field_starts_at_one() {
        let dir = tempfile::tempdir().unwrap();
        let meta_path = dir.path().join("meta.json");
        std::fs::write(&meta_path, r#"{"version":"1.0"}"#).unwrap();

        let count = increment_invocation_count(dir.path()).unwrap();
        assert_eq!(count, 1);
    }
}
