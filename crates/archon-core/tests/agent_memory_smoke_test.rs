//! Smoke test: Agent Memory Lifecycle end-to-end.
//!
//! Exercises the full memory lifecycle: dir resolution → dir creation →
//! MEMORY.md read → truncation → prompt building → extraction gating.

use std::path::Path;

use archon_core::agents::definition::AgentMemoryScope;
use archon_core::agents::memory::{
    ExtractionState, ensure_memory_dir_exists, get_agent_memory_dir, load_agent_memory_prompt,
    scan_memory_files, should_extract, truncate_entrypoint_content,
};

/// Full lifecycle: create dir → write MEMORY.md → load prompt → verify content.
#[tokio::test]
async fn smoke_full_memory_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path();

    // 1. Resolve memory dir
    let dir = get_agent_memory_dir("smoke-agent", &AgentMemoryScope::Project, cwd);
    assert!(
        dir.to_string_lossy()
            .contains(".archon/agent-memory/smoke-agent")
    );

    // 2. Create directory
    ensure_memory_dir_exists(&dir).await;
    assert!(dir.exists());

    // 3. Write MEMORY.md
    std::fs::write(
        dir.join("MEMORY.md"),
        "- [Role](role.md) — Senior Rust developer\n- [Prefs](prefs.md) — Prefers terse output",
    )
    .unwrap();

    // 4. Load prompt
    let prompt =
        load_agent_memory_prompt("smoke-agent", Some(&AgentMemoryScope::Project), cwd).unwrap();

    // 5. Verify prompt contains lifecycle elements
    assert!(prompt.contains("# auto memory"), "header");
    assert!(prompt.contains("Senior Rust developer"), "existing memory");
    assert!(prompt.contains("Prefers terse output"), "existing memory");
    assert!(prompt.contains("<name>user</name>"), "taxonomy");
    assert!(prompt.contains("What NOT to save"), "exclusions");
    assert!(prompt.contains("Step 1"), "how-to-save");
}

/// Verify None scope produces no prompt (AC-101).
#[test]
fn smoke_none_scope_no_prompt() {
    let result = load_agent_memory_prompt("agent", None, Path::new("/tmp"));
    assert!(result.is_none());
}

/// Verify extraction gating with realistic message sequence.
#[test]
fn smoke_extraction_gating_realistic() {
    let dir = Path::new("/tmp/.archon/agent-memory/smoke");
    let mut state = ExtractionState::default();

    // Turn 1: normal message → should extract
    let msgs = vec![serde_json::json!({"uuid": "turn-1", "role": "user", "content": "hello"})];
    assert!(should_extract(false, &mut state, &msgs, dir));

    // Turn 2: agent wrote to memory → should NOT extract
    let msgs_with_write = vec![serde_json::json!({
        "uuid": "turn-2",
        "role": "assistant",
        "content": [{
            "type": "tool_use",
            "name": "Write",
            "input": { "file_path": "/tmp/.archon/agent-memory/smoke/role.md", "content": "x" }
        }]
    })];
    assert!(!should_extract(false, &mut state, &msgs_with_write, dir));
}

/// Verify truncation with realistic content.
#[test]
fn smoke_truncation_realistic() {
    // Simulate a MEMORY.md with 250 entries (over 200 line limit)
    let content = (0..250)
        .map(|i| format!("- [Item {i}](item_{i}.md) — Description for item {i}"))
        .collect::<Vec<_>>()
        .join("\n");

    let result = truncate_entrypoint_content(&content, 200, 25_000);
    assert!(result.contains("WARNING"));
    assert!(result.contains("250 lines"));
    // Should have exactly 200 content lines
    let before_warning = result.split("\n\n> WARNING").next().unwrap();
    assert_eq!(before_warning.lines().count(), 200);
}

/// Verify scan_memory_files with realistic directory.
#[tokio::test]
async fn smoke_scan_memory_files_realistic() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("MEMORY.md"), "index").unwrap();
    std::fs::write(tmp.path().join("user_role.md"), "role info").unwrap();
    std::fs::write(tmp.path().join("feedback_testing.md"), "test prefs").unwrap();

    let manifest = scan_memory_files(tmp.path()).await;
    assert!(manifest.contains("MEMORY.md"));
    assert!(manifest.contains("user_role.md"));
    assert!(manifest.contains("feedback_testing.md"));
}
