use super::*;

#[test]
fn has_memory_writes_no_messages() {
    let dir = Path::new("/tmp/.archon/agent-memory/test");
    assert!(!has_memory_writes_since(&[], None, dir));
}

#[test]
fn has_memory_writes_detects_write_to_memory_dir() {
    let dir = Path::new("/tmp/.archon/agent-memory/test");
    let messages = vec![serde_json::json!({
        "role": "assistant",
        "content": [{
            "type": "tool_use",
            "name": "Write",
            "input": {
                "file_path": "/tmp/.archon/agent-memory/test/role.md",
                "content": "some memory"
            }
        }]
    })];
    assert!(has_memory_writes_since(&messages, None, dir));
}

#[test]
fn has_memory_writes_detects_edit_to_memory_dir() {
    let dir = Path::new("/tmp/.archon/agent-memory/test");
    let messages = vec![serde_json::json!({
        "role": "assistant",
        "content": [{
            "type": "tool_use",
            "name": "Edit",
            "input": {
                "file_path": "/tmp/.archon/agent-memory/test/MEMORY.md",
                "old_string": "old",
                "new_string": "new"
            }
        }]
    })];
    assert!(has_memory_writes_since(&messages, None, dir));
}

#[test]
fn has_memory_writes_ignores_write_outside_memory_dir() {
    let dir = Path::new("/tmp/.archon/agent-memory/test");
    let messages = vec![serde_json::json!({
        "role": "assistant",
        "content": [{
            "type": "tool_use",
            "name": "Write",
            "input": {
                "file_path": "/tmp/src/main.rs",
                "content": "code"
            }
        }]
    })];
    assert!(!has_memory_writes_since(&messages, None, dir));
}

#[test]
fn has_memory_writes_respects_cursor() {
    let dir = Path::new("/tmp/.archon/agent-memory/test");
    let messages = vec![
        serde_json::json!({
            "uuid": "msg-001",
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Write",
                "input": { "file_path": "/tmp/.archon/agent-memory/test/old.md", "content": "x" }
            }]
        }),
        serde_json::json!({
            "uuid": "msg-002",
            "role": "user",
            "content": "next turn"
        }),
    ];
    assert!(!has_memory_writes_since(&messages, Some("msg-001"), dir));
}

#[test]
fn has_memory_writes_detects_after_cursor() {
    let dir = Path::new("/tmp/.archon/agent-memory/test");
    let messages = vec![
        serde_json::json!({
            "uuid": "msg-001",
            "role": "user",
            "content": "first"
        }),
        serde_json::json!({
            "uuid": "msg-002",
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Write",
                "input": { "file_path": "/tmp/.archon/agent-memory/test/new.md", "content": "x" }
            }]
        }),
    ];
    assert!(has_memory_writes_since(&messages, Some("msg-001"), dir));
}

#[test]
fn should_extract_false_for_subagent() {
    let dir = Path::new("/tmp/test");
    let mut state = ExtractionState::default();
    let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
    assert!(!should_extract(true, &mut state, &msgs, dir));
}

#[test]
fn should_extract_true_for_main_session() {
    let dir = Path::new("/tmp/test");
    let mut state = ExtractionState::default();
    let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];
    assert!(should_extract(false, &mut state, &msgs, dir));
}

#[test]
fn should_extract_throttles_by_interval() {
    let dir = Path::new("/tmp/test");
    let mut state = ExtractionState::with_interval(3);
    let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];

    assert!(!should_extract(false, &mut state, &msgs, dir));
    assert!(!should_extract(false, &mut state, &msgs, dir));
    assert!(should_extract(false, &mut state, &msgs, dir));
    assert!(!should_extract(false, &mut state, &msgs, dir));
}

#[test]
fn should_extract_false_when_agent_wrote_memory() {
    let dir = Path::new("/tmp/.archon/agent-memory/test");
    let mut state = ExtractionState::default();
    let msgs = vec![serde_json::json!({
        "role": "assistant",
        "content": [{
            "type": "tool_use",
            "name": "Write",
            "input": { "file_path": "/tmp/.archon/agent-memory/test/role.md", "content": "x" }
        }]
    })];
    assert!(!should_extract(false, &mut state, &msgs, dir));
}

#[test]
fn should_extract_false_for_empty_messages() {
    let dir = Path::new("/tmp/test");
    let mut state = ExtractionState::default();
    assert!(!should_extract(false, &mut state, &[], dir));
}

#[test]
fn extraction_state_default_interval_is_one() {
    let state = ExtractionState::default();
    assert_eq!(state.extraction_interval, 1);
    assert_eq!(state.turns_since_last_extraction, 0);
    assert!(state.last_memory_message_uuid.is_none());
}

#[tokio::test]
async fn scan_memory_files_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let result = scan_memory_files(tmp.path()).await;
    assert_eq!(result, "(no files)");
}

#[tokio::test]
async fn scan_memory_files_lists_md_only() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("role.md"), "memory").unwrap();
    std::fs::write(tmp.path().join("notes.md"), "memory").unwrap();
    std::fs::write(tmp.path().join("data.json"), "{}").unwrap();
    let result = scan_memory_files(tmp.path()).await;
    assert!(result.contains("role.md"));
    assert!(result.contains("notes.md"));
    assert!(!result.contains("data.json"));
}

#[tokio::test]
async fn scan_memory_files_nonexistent_dir() {
    let result = scan_memory_files(Path::new("/nonexistent/dir")).await;
    assert_eq!(result, "(no files)");
}
