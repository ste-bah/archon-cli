use super::helpers::{MockMemory, NullMemory};
use super::*;

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
    let results = load_agent_memory(
        "test-agent",
        &queries,
        &memory,
        Some(&AgentMemoryScope::User),
    );
    assert!(results.is_empty());
}

#[test]
fn load_agent_memory_none_scope_returns_empty() {
    let memory = NullMemory;
    let queries = vec!["test query".to_string()];
    let results = load_agent_memory("test-agent", &queries, &memory, None);
    assert!(
        results.is_empty(),
        "AC-101: None scope = no persistent memory"
    );
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
